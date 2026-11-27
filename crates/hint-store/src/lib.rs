//! 本地会话存储：SQLite。
//!
//! 存储面试过程中的转录段落与答案，供复盘页展示和 Markdown 导出。
//!
//! 设计要点：
//! - 不存原始音频，只存文本与延迟指标；
//! - `segments.start_ms` / `end_ms` 是相对会话开始时刻的毫秒偏移，
//!   `sessions.started_at` / `ended_at` 是 Unix 毫秒时间戳；
//! - 表结构通过 `PRAGMA user_version` 做版本化迁移，打开旧库会自动升级；
//! - 删除会话时依赖外键级联清掉段落与答案（连接创建时开启 `foreign_keys`）。
//!
//! 典型用法：
//! ```no_run
//! # fn main() -> hint_store::Result<()> {
//! let store = hint_store::Store::open("hint.db")?;
//! let session = store.create_session("openai", "deepseek-chat", None)?;
//! let segment = store.add_segment(
//!     session,
//!     hint_store::CHANNEL_SYSTEM,
//!     0,
//!     2_400,
//!     "你能说说这个项目的难点吗？",
//!     false,
//!     false,
//! )?;
//! store.add_answer(
//!     session,
//!     Some(segment),
//!     "难点主要有三点……",
//!     "deepseek-chat",
//!     hint_store::AnswerMetrics::default(),
//! )?;
//! store.end_session(session)?;
//! println!("{}", store.export_markdown(session)?);
//! # Ok(())
//! # }
//! ```

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Local};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use thiserror::Error;

/// 通道名：麦克风（候选人本人）。
pub const CHANNEL_MIC: &str = "mic";

/// 通道名：系统声（面试官）。
pub const CHANNEL_SYSTEM: &str = "system";

/// 当前 schema 版本。新增表结构变更时追加迁移语句并递增。
const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at  INTEGER NOT NULL,
    ended_at    INTEGER,
    provider_id TEXT    NOT NULL,
    model_id    TEXT    NOT NULL,
    note        TEXT
);

CREATE TABLE IF NOT EXISTS segments (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    channel    TEXT    NOT NULL,
    start_ms   INTEGER NOT NULL,
    end_ms     INTEGER NOT NULL,
    text       TEXT    NOT NULL,
    is_echo    INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_segments_session ON segments(session_id, start_ms);

CREATE TABLE IF NOT EXISTS answers (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id          INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    question_segment_id INTEGER REFERENCES segments(id) ON DELETE SET NULL,
    text                TEXT    NOT NULL,
    model_id            TEXT    NOT NULL,
    ttft_ms             INTEGER,
    total_ms            INTEGER,
    token_in            INTEGER,
    token_out           INTEGER
);

CREATE INDEX IF NOT EXISTS idx_answers_session ON answers(session_id);

CREATE TABLE IF NOT EXISTS profiles (
    kind       TEXT    NOT NULL,
    name       TEXT    NOT NULL,
    content    TEXT    NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (kind, name)
);
"#;

/// 按顺序执行的迁移脚本，下标即目标版本。
///
/// V2：segments 补 `recovered` 列——断线补录的段落在 V1 只落库文本，
/// 复盘页「补录」图例永远无数据。`ALTER TABLE ... ADD COLUMN` 幂等
/// 由 user_version 保证（只会执行一次）。
const SCHEMA_V2: &str = "ALTER TABLE segments ADD COLUMN recovered INTEGER NOT NULL DEFAULT 0;";

const MIGRATIONS: &[&str] = &[SCHEMA_V1, SCHEMA_V2];

/// 存储层错误。
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("数据库错误：{0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("未找到会话 {0}")]
    SessionNotFound(i64),
}

/// 存储层 Result 别名。
pub type Result<T> = std::result::Result<T, StoreError>;

/// 数据健康摘要（`--doctor` 用）。
///
/// 与 `PRAGMA integrity_check` 的分工：那个查「文件结构有没有坏」，这个查
/// 「数据本身合不合理」。两者都要看——结构完好但满库孤儿数据的库，用户感受
/// 就是「记录不对劲」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataHealth {
    pub sessions: i64,
    pub segments: i64,
    pub answers: i64,
    /// 指向已不存在会话的转录段落（正常应恒为 0）。
    pub orphan_segments: i64,
    /// 指向已不存在会话的答案（正常应恒为 0）。
    pub orphan_answers: i64,
    /// `ended_at IS NULL` 的会话：进程被强杀留下的痕迹。
    pub unfinished: i64,
    /// 没有转录也没有答案的会话。
    pub empty: i64,
}

/// 一次面试会话。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: i64,
    /// Unix 毫秒时间戳。
    pub started_at: i64,
    /// Unix 毫秒时间戳；未结束时为 `None`。
    pub ended_at: Option<i64>,
    pub provider_id: String,
    pub model_id: String,
    pub note: Option<String>,
}

/// 会话列表项，附带段落与答案条数，供复盘页列表展示。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub session: Session,
    pub segment_count: i64,
    pub answer_count: i64,
}

/// 一条转录段落。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub id: i64,
    pub session_id: i64,
    pub channel: String,
    /// 相对会话开始的毫秒偏移。
    pub start_ms: i64,
    /// 相对会话开始的毫秒偏移。
    pub end_ms: i64,
    pub text: String,
    pub is_echo: bool,
    /// 断线恢复后补录的段落（前端复盘页标「补录」）。
    pub recovered: bool,
}

impl Segment {
    /// 通道对应的中文标签。
    pub fn speaker_label(&self) -> &str {
        channel_label(&self.channel)
    }
}

/// 通道名转中文标签；未知通道原样返回。
///
/// 系统声通道标签用「提问」而不是「面试官」：这是产品口径——界面上不出现
/// 「面试官」这个暴露使用场景的词，导出与复盘里也一致。
pub fn channel_label(channel: &str) -> &str {
    match channel {
        CHANNEL_MIC => "我",
        CHANNEL_SYSTEM => "提问",
        other => other,
    }
}

/// 一条生成的答案。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub id: i64,
    pub session_id: i64,
    /// 触发生成的提问段落；配对失败时为 `None`。
    pub question_segment_id: Option<i64>,
    pub text: String,
    pub model_id: String,
    /// 首 token 耗时（毫秒）。
    pub ttft_ms: Option<i64>,
    /// 总耗时（毫秒）。
    pub total_ms: Option<i64>,
    pub token_in: Option<i64>,
    pub token_out: Option<i64>,
}

/// 生成答案时的可选延迟/用量指标。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnswerMetrics {
    pub ttft_ms: Option<i64>,
    pub total_ms: Option<i64>,
    pub token_in: Option<i64>,
    pub token_out: Option<i64>,
}

/// 本地资料（简历或项目），`kind` 区分类型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// 资料类型：`resume` | `project`。
    pub kind: String,
    pub name: String,
    pub content: String,
    pub updated_at: i64,
}

/// 会话详情：会话本身 + 全部段落 + 全部答案。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    pub session: Session,
    pub segments: Vec<Segment>,
    pub answers: Vec<Answer>,
}

/// SQLite 存储句柄。
///
/// 内部持有一个 [`rusqlite::Connection`]；不是 `Sync`，跨线程使用时请包
/// `Mutex` 或把 `Store` 放在单独的工作线程里。
pub struct Store {
    conn: Connection,
}

impl Store {
    /// 打开（或创建）数据库文件，自动建表并升级 schema。
    ///
    /// 传 `":memory:"` 或使用 [`Store::open_in_memory`] 可得到内存库（测试用）。
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = if path.as_ref() == Path::new(":memory:") {
            Connection::open_in_memory()?
        } else {
            Connection::open(path)?
        };
        Self::from_connection(conn)
    }

    /// 打开内存数据库，仅用于测试与临时场景。
    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;
        // WAL 对文件库有并发收益；内存库返回 "memory"，忽略结果即可。
        let _ = conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()));
        migrate(&conn)?;
        Ok(Self { conn })
    }

    /// 读取底层连接，供集成方执行只读查询（如统计）。
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    // ----- 会话 -----

    /// 创建会话并返回 id。`started_at` 取当前时间。
    pub fn create_session(
        &self,
        provider_id: &str,
        model_id: &str,
        note: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO sessions (started_at, provider_id, model_id, note) VALUES (?, ?, ?, ?)",
            params![now_ms(), provider_id, model_id, note],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 标记会话结束。重复调用会刷新结束时间。
    pub fn end_session(&self, id: i64) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE sessions SET ended_at = ? WHERE id = ?",
            params![now_ms(), id],
        )?;
        if changed == 0 {
            return Err(StoreError::SessionNotFound(id));
        }
        Ok(())
    }

    /// 按开始时间倒序列出最近 `limit` 个会话（含段落/答案计数）。
    pub fn list_sessions(&self, limit: usize) -> Result<Vec<SessionSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.started_at, s.ended_at, s.provider_id, s.model_id, s.note,
                    (SELECT COUNT(*) FROM segments WHERE session_id = s.id),
                    (SELECT COUNT(*) FROM answers  WHERE session_id = s.id)
             FROM sessions s
             ORDER BY s.started_at DESC, s.id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(SessionSummary {
                session: Session {
                    id: row.get(0)?,
                    started_at: row.get(1)?,
                    ended_at: row.get(2)?,
                    provider_id: row.get(3)?,
                    model_id: row.get(4)?,
                    note: row.get(5)?,
                },
                segment_count: row.get(6)?,
                answer_count: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 按**转录或答案内容**搜索会话，返回命中的会话 id（按最近优先）。
    ///
    /// 为什么需要：会话攒到几十条之后，列表只能按时间倒序翻——用户记得的是
    /// 「问过 kafka 的那次面试」，不是「10 月 17 日 15:37 那次」。没有内容搜索
    /// 就只能一条条点开找。
    ///
    /// 搜**两个**表而不只是转录：用户也会记得「当时 AI 给的那个方案」，那时关键词
    /// 只出现在答案里。只搜转录会让这类查找落空，而界面上看不出「搜过了但没搜答案」。
    ///
    /// 实现用 `LIKE '%词%'`：
    /// - 转录总量是「几十场 × 几百段」，几万行的 LIKE 全扫描在桌面场景是毫秒级；
    /// - 不引入 FTS5（SQLite 全文索引）：它要求额外的虚表与触发器，还要处理
    ///   中文分词——本项目的中文检索一直是二元组匹配，与 `profile::tokenize`
    ///   同一思路，不需要分词器。
    ///
    /// `query` 里的 `%` `_` `\` 会被转义，避免用户输入 `%` 匹配到全部。
    pub fn search_sessions(&self, query: &str, limit: usize) -> Result<Vec<i64>> {
        let needle = query.trim();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        // LIKE 的特殊字符：用 `\` 转义，并在 SQL 里声明 ESCAPE。
        let escaped = needle
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{escaped}%");
        // UNION 而不是两次查询再合并：SQL 这边一次扫完，DISTINCT 顺带去掉
        // 「同一会话里转录和答案都命中」的重复行。
        //
        // `note` 也进搜索：备注是用户给会话起的名字（「腾讯一面」「字节二面」），
        // 那是他最可能用来找回某场记录的关键词。真机验证过的缺口——备注写进去了
        // （详情能读到），但搜「独特备注ZZTOP」命中 0 条，而搜正文同一会话能命中。
        // 用户给会话起了名却搜不到，比没起名更让人困惑。
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT session_id FROM (
                 SELECT id AS session_id FROM sessions WHERE note LIKE ?1 ESCAPE '\\'
                 UNION
                 SELECT session_id FROM segments WHERE text LIKE ?1 ESCAPE '\\'
                 UNION
                 SELECT session_id FROM answers  WHERE text LIKE ?1 ESCAPE '\\'
             )
             ORDER BY session_id DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit as i64], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 给会话写备注（显示名）。
    ///
    /// 备注是列表标题的**首选**（`note > started_at > id`），但此前没有任何写入
    /// 入口——41 条会话在界面上全是「2026-11-26 15:37:30」这种时间戳，用户想标
    /// 「字节二面」「腾讯一面」做不到，只能靠记时间。
    ///
    /// `note` 传空串等于清除备注（回落到时间戳）。返回是否真的更新到记录
    /// （false 说明 id 不存在——调用方要能区分「清空成功」与「这个会话没了」）。
    pub fn set_session_note(&self, id: i64, note: &str) -> Result<bool> {
        // 空串存 NULL 而不是空字符串：`note` 列是 `Option<String>`，语义上
        // 「没有备注」应该只有**一种**表示。存成 `Some("")` 会让读侧多一条分支
        // （空串也要当没有），而且列表标题的回落逻辑会先命中空串再落到时间戳。
        let trimmed = note.trim();
        let value: Option<&str> = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        };
        let changed = self.conn.execute(
            "UPDATE sessions SET note = ?1 WHERE id = ?2",
            params![value, id],
        )?;
        Ok(changed > 0)
    }

    /// 这个会话有没有任何内容（转录或答案）。
    ///
    /// 用途：会话结束时要判断「该不该留下这条记录」。用户点了开始又立刻结束、
    /// 或麦克风没权限导致一句都没识别到时，留一条空记录只会在历史里变成噪声。
    ///
    /// 用 `EXISTS` 而不是 count：只要知道有没有，不需要数多少，命中一条就能返回。
    pub fn session_has_content(&self, id: i64) -> Result<bool> {
        let found: i64 = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM segments WHERE session_id = ?1)
                 OR EXISTS(SELECT 1 FROM answers  WHERE session_id = ?1)",
            params![id],
            |row| row.get(0),
        )?;
        Ok(found != 0)
    }

    /// 没有任何内容的会话条数（转录与答案都为空）。
    ///
    /// 与 [`Self::session_has_content`] 同一判据的集合版本：删之前先数，界面才能
    /// 问「确定清理 N 条吗」而不是盲删。
    pub fn count_empty_sessions(&self) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM sessions s
             WHERE NOT EXISTS(SELECT 1 FROM segments WHERE session_id = s.id)
               AND NOT EXISTS(SELECT 1 FROM answers  WHERE session_id = s.id)",
            [],
            |row| row.get(0),
        )?)
    }

    /// 删除全部空会话，返回删除条数。
    ///
    /// 判据与 [`Self::count_empty_sessions`] 逐字一致（同样两个 NOT EXISTS）——
    /// 两处写不同判据的话，「数出来 33 条、删掉 30 条」这种不一致会让用户以为
    /// 有 3 条删不掉。
    pub fn delete_empty_sessions(&self) -> Result<usize> {
        let changed = self.conn.execute(
            "DELETE FROM sessions
             WHERE NOT EXISTS(SELECT 1 FROM segments WHERE session_id = sessions.id)
               AND NOT EXISTS(SELECT 1 FROM answers  WHERE session_id = sessions.id)",
            [],
        )?;
        Ok(changed)
    }

    /// 会话总数。
    ///
    /// 列表接口有上限（当前 200 条），超过之后**必须**能告诉用户「还有更多」——
    /// 否则界面上看不出记录被截断，用户会以为早先的会话丢了。
    pub fn count_sessions(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?)
    }

    /// 数据健康摘要：给 `--doctor` 用，用一眼看清「库里有几成是有效数据」。
    ///
    /// ## 为什么需要
    ///
    /// `PRAGMA integrity_check` 只回答「文件结构有没有损坏」，答不了「数据本身
    /// 合不合理」。真机上可能出现结构完好、但内容不对劲的库：
    ///
    /// - **孤儿数据**：`segments`/`answers` 指向已不存在的会话。正常路径不会产生
    ///   （外键级联会清），但历史版本、外部工具改库、或外键未开启的旧连接都可能
    ///   留下；它们占空间、拖慢查询，且永远显示不出来。
    /// - **未结束的会话**：进程被强杀时 `ended_at` 为 NULL。数量多说明崩溃频繁，
    ///   这是「用户从没提过但一直在发生」的信号。
    /// - **空会话**：没有转录也没有答案。数量多说明「开了会话又马上停」的操作很常见。
    ///
    /// 这些数字放在诊断报告里，用户反馈「记录不对劲」时就有据可查，而不是只能
    /// 靠人问「你有几条会话」。
    pub fn data_health(&self) -> Result<DataHealth> {
        let sessions: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?;
        let segments: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM segments", [], |row| row.get(0))?;
        let answers: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM answers", [], |row| row.get(0))?;
        let orphan_segments: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM segments WHERE session_id NOT IN (SELECT id FROM sessions)",
            [],
            |row| row.get(0),
        )?;
        let orphan_answers: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM answers WHERE session_id NOT IN (SELECT id FROM sessions)",
            [],
            |row| row.get(0),
        )?;
        let unfinished: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM sessions WHERE ended_at IS NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(DataHealth {
            sessions,
            segments,
            answers,
            orphan_segments,
            orphan_answers,
            unfinished,
            empty: self.count_empty_sessions()?,
        })
    }

    /// 读取单个会话。
    pub fn get_session(&self, id: i64) -> Result<Session> {
        self.conn
            .query_row(
                "SELECT id, started_at, ended_at, provider_id, model_id, note
                 FROM sessions WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Session {
                        id: row.get(0)?,
                        started_at: row.get(1)?,
                        ended_at: row.get(2)?,
                        provider_id: row.get(3)?,
                        model_id: row.get(4)?,
                        note: row.get(5)?,
                    })
                },
            )
            .optional()?
            .ok_or(StoreError::SessionNotFound(id))
    }

    /// 会话详情：会话 + 段落（按开始时间排序）+ 答案（按生成顺序）。
    pub fn session_detail(&self, id: i64) -> Result<SessionDetail> {
        let session = self.get_session(id)?;
        Ok(SessionDetail {
            session,
            segments: self.segments_of(id)?,
            answers: self.answers_of(id)?,
        })
    }

    /// 删除会话及其全部段落、答案。返回是否真的删除了记录。
    pub fn delete_session(&self, id: i64) -> Result<bool> {
        let changed = self
            .conn
            .execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
        Ok(changed > 0)
    }

    /// 删除全部会话，返回删除条数。
    pub fn delete_all_sessions(&self) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM sessions", [])?)
    }

    /// 删除开始时间早于 `days` 天前的会话，返回删除条数。
    ///
    /// `days == 0` 表示「不清理」（返回 0）：否则截止时间就是当前时刻，
    /// 刚创建、甚至同一毫秒内的会话都可能被误删。
    /// 删除会话时依赖外键级联清掉其段落与答案。
    pub fn cleanup_older_than_days(&self, days: u32) -> Result<usize> {
        if days == 0 {
            return Ok(0);
        }
        let cutoff = now_ms() - i64::from(days) * 24 * 60 * 60 * 1_000;
        Ok(self.conn.execute(
            "DELETE FROM sessions WHERE started_at < ?1",
            params![cutoff],
        )?)
    }

    // ----- 段落与答案 -----

    /// 追加一条转录段落，返回段落 id。
    ///
    /// `start_ms` / `end_ms` 为相对会话开始的毫秒偏移；`is_echo` 由
    /// `hint-core` 的 `is_echo` 判定后传入，回声段也应落库（便于复盘），
    /// 但不参与提示词上下文。`recovered` 标记断线恢复后的补录段落。
    /// 参数是对 SQLite 列的一一映射，聚合成 struct 反而让调用点更啰嗦，
    /// 豁免 too_many_arguments。
    #[allow(clippy::too_many_arguments)]
    pub fn add_segment(
        &self,
        session_id: i64,
        channel: &str,
        start_ms: i64,
        end_ms: i64,
        text: &str,
        is_echo: bool,
        recovered: bool,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO segments (session_id, channel, start_ms, end_ms, text, is_echo, recovered)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                session_id,
                channel,
                start_ms,
                end_ms,
                text,
                is_echo as i64,
                recovered as i64
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 追加一条答案，返回答案 id。
    ///
    /// `question_segment_id` 应与 `session_id` 属于同一会话；跨会话的 id 不会
    /// 被外键拦截，调用方需自行保证。
    pub fn add_answer(
        &self,
        session_id: i64,
        question_segment_id: Option<i64>,
        text: &str,
        model_id: &str,
        metrics: AnswerMetrics,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO answers
                (session_id, question_segment_id, text, model_id,
                 ttft_ms, total_ms, token_in, token_out)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                session_id,
                question_segment_id,
                text,
                model_id,
                metrics.ttft_ms,
                metrics.total_ms,
                metrics.token_in,
                metrics.token_out
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 会话的全部段落，按 `start_ms` 升序。
    pub fn segments_of(&self, session_id: i64) -> Result<Vec<Segment>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, channel, start_ms, end_ms, text, is_echo, recovered
             FROM segments WHERE session_id = ?1 ORDER BY start_ms, id",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(Segment {
                id: row.get(0)?,
                session_id: row.get(1)?,
                channel: row.get(2)?,
                start_ms: row.get(3)?,
                end_ms: row.get(4)?,
                text: row.get(5)?,
                is_echo: row.get::<_, i64>(6)? != 0,
                recovered: row.get::<_, i64>(7)? != 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 会话的全部答案，按生成顺序。
    pub fn answers_of(&self, session_id: i64) -> Result<Vec<Answer>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, question_segment_id, text, model_id,
                    ttft_ms, total_ms, token_in, token_out
             FROM answers WHERE session_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(Answer {
                id: row.get(0)?,
                session_id: row.get(1)?,
                question_segment_id: row.get(2)?,
                text: row.get(3)?,
                model_id: row.get(4)?,
                ttft_ms: row.get(5)?,
                total_ms: row.get(6)?,
                token_in: row.get(7)?,
                token_out: row.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ----- 资料 -----

    /// 保存（或覆盖）一条资料。`kind` 建议用 `resume` / `project`。
    pub fn save_profile(&self, kind: &str, name: &str, content: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO profiles (kind, name, content, updated_at) VALUES (?, ?, ?, ?)
             ON CONFLICT(kind, name)
             DO UPDATE SET content = excluded.content, updated_at = excluded.updated_at",
            params![kind, name, content, now_ms()],
        )?;
        Ok(())
    }

    /// 读取一条资料；不存在时返回 `None`。
    pub fn get_profile(&self, kind: &str, name: &str) -> Result<Option<Profile>> {
        Ok(self
            .conn
            .query_row(
                "SELECT kind, name, content, updated_at
                 FROM profiles WHERE kind = ?1 AND name = ?2",
                params![kind, name],
                |row| {
                    Ok(Profile {
                        kind: row.get(0)?,
                        name: row.get(1)?,
                        content: row.get(2)?,
                        updated_at: row.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// 列出某一类资料，按更新时间倒序。
    pub fn list_profiles(&self, kind: &str) -> Result<Vec<Profile>> {
        let mut stmt = self.conn.prepare(
            "SELECT kind, name, content, updated_at
             FROM profiles WHERE kind = ?1 ORDER BY updated_at DESC, name",
        )?;
        let rows = stmt.query_map(params![kind], |row| {
            Ok(Profile {
                kind: row.get(0)?,
                name: row.get(1)?,
                content: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 删除一条资料，返回是否真的删除了记录。
    pub fn delete_profile(&self, kind: &str, name: &str) -> Result<bool> {
        let changed = self.conn.execute(
            "DELETE FROM profiles WHERE kind = ?1 AND name = ?2",
            params![kind, name],
        )?;
        Ok(changed > 0)
    }

    // ----- 导出 -----

    /// 把一次会话导出为 Markdown 文本：转录与问答按时间轴交错排列。
    ///
    /// 无法关联到具体提问的答案会单独列在文末「其他回答」一节。
    pub fn export_markdown(&self, id: i64) -> Result<String> {
        let detail = self.session_detail(id)?;
        let session = &detail.session;

        let mut out = String::new();
        out.push_str(&format!("# 面试记录 #{}\n\n", session.id));
        out.push_str(&format!(
            "- 开始时间：{}\n",
            format_datetime(session.started_at)
        ));
        match session.ended_at {
            Some(ended_at) => out.push_str(&format!("- 结束时间：{}\n", format_datetime(ended_at))),
            None => out.push_str("- 结束时间：（未结束）\n"),
        }
        out.push_str(&format!(
            "- 模型：{} / {}\n",
            session.provider_id, session.model_id
        ));
        if let Some(note) = session
            .note
            .as_deref()
            .filter(|note| !note.trim().is_empty())
        {
            out.push_str(&format!("- 备注：{}\n", note.trim()));
        }
        out.push('\n');

        if detail.segments.is_empty() && detail.answers.is_empty() {
            out.push_str("_本次会话没有转录内容。_\n");
            return Ok(out);
        }

        // 小节标题**按需输出**：没有转录就不写「转录与问答」。
        //
        // 真机验证过的形态：会话 42/43 只有答案、没有任何转录段落（那几次是调试时
        // 直接点「问一句」，没开会）。导出文件里会出现一个空标题：
        //
        //     ## 转录与问答
        //
        //     ## 其他回答
        //     **回答**（首字 2.3s · 总耗时 4.2s）…
        //
        // 空的 `## 转录与问答` 读起来像「转录内容丢了」——用户看到第一反应是数据
        // 出问题，而不是「这场本来就没转录」。
        if !detail.segments.is_empty() {
            out.push_str("## 转录与问答\n\n");
        }
        // 同一提问可能有多次「重新生成」产生的多条答案，必须全部导出；
        // 按答案 id（生成顺序）分组，直接遍历一次，避免 O(n²) 的 find。
        let mut answers_by_segment: std::collections::HashMap<i64, Vec<&Answer>> =
            std::collections::HashMap::new();
        let mut orphan_answers: Vec<&Answer> = Vec::new();
        for answer in &detail.answers {
            match answer.question_segment_id {
                Some(segment_id) => answers_by_segment
                    .entry(segment_id)
                    .or_default()
                    .push(answer),
                None => orphan_answers.push(answer),
            }
        }

        for segment in &detail.segments {
            let time = format_clock(session.started_at + segment.start_ms);
            out.push_str(&format!(
                "**{}**（{}）：{}\n",
                segment.speaker_label(),
                time,
                segment.text.trim()
            ));
            if segment.is_echo {
                out.push_str("（回声，已排除）\n");
            }
            out.push('\n');

            if let Some(answers) = answers_by_segment.get(&segment.id) {
                for answer in answers {
                    render_answer(&mut out, answer);
                }
            }
        }

        // 关联到「不属于本会话的段落」的答案也应出现在导出里，而不是凭空消失。
        for (segment_id, answers) in &answers_by_segment {
            if !detail
                .segments
                .iter()
                .any(|segment| segment.id == *segment_id)
            {
                orphan_answers.extend(answers.iter().copied());
            }
        }
        orphan_answers.sort_by_key(|answer| answer.id);
        if !orphan_answers.is_empty() {
            out.push_str("## 其他回答\n\n");
            for answer in orphan_answers {
                render_answer(&mut out, answer);
            }
        }

        Ok(out)
    }
}

/// 渲染一条答案到 Markdown。
fn render_answer(out: &mut String, answer: &Answer) {
    let mut meta: Vec<String> = Vec::new();
    if let Some(ttft) = answer.ttft_ms {
        meta.push(format!("首字 {}", format_seconds(ttft)));
    }
    if let Some(total) = answer.total_ms {
        meta.push(format!("总耗时 {}", format_seconds(total)));
    }
    if answer.token_in.is_some() || answer.token_out.is_some() {
        meta.push(format!(
            "tokens {}/{}",
            answer.token_in.unwrap_or(0),
            answer.token_out.unwrap_or(0)
        ));
    }
    if meta.is_empty() {
        out.push_str("**回答**\n\n");
    } else {
        out.push_str(&format!("**回答**（{}）\n\n", meta.join(" · ")));
    }
    out.push_str(answer.text.trim());
    out.push_str("\n\n");
}

/// 把毫秒耗时格式化为秒，保留一位小数。
fn format_seconds(ms: i64) -> String {
    format!("{:.1}s", ms as f64 / 1000.0)
}

/// Unix 毫秒 → 本地时间 `YYYY-MM-DD HH:MM:SS`；越界时退回原始数字。
fn format_datetime(ms: i64) -> String {
    match DateTime::from_timestamp_millis(ms) {
        Some(utc) => utc
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
        None => ms.to_string(),
    }
}

/// Unix 毫秒 → 本地时间 `HH:MM:SS`。
fn format_clock(ms: i64) -> String {
    match DateTime::from_timestamp_millis(ms) {
        Some(utc) => utc.with_timezone(&Local).format("%H:%M:%S").to_string(),
        None => ms.to_string(),
    }
}

/// 当前 Unix 毫秒时间戳；系统时间早于 1970 时退化为 0（不 panic）。
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// 按 `PRAGMA user_version` 依次执行未应用的迁移。
fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        let next_version = index as i64 + 1;
        conn.execute_batch(&format!(
            "BEGIN;\n{sql}\nPRAGMA user_version = {next_version};\nCOMMIT;"
        ))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 没有转录时不该输出空的「转录与问答」标题。
    ///
    /// 真机形态：会话 42/43 只有答案、没有转录段落（调试时直接点「问一句」，没开会）。
    /// 导出里会出现空的 `## 转录与问答`——读起来像「转录内容丢了」，用户第一反应
    /// 是数据出问题，而不是「这场本来就没转录」。
    /// 导出的**特殊内容**要能安全通过。
    ///
    /// 用户转录和答案里可能出现换行、反引号、管道符、表格这些会破坏 Markdown
    /// 结构的字符。导出是把内容**原样**写进 Markdown 的（`segment.text`、
    /// `answer.text` 直接拼接），不做转义——这是对的：用户说的话、模型给的答案
    /// 就是要原样保留。
    ///
    /// 但「原样」不等于「不管结构」。多行答案如果直接内嵌，会把后面的标题、
    /// 下一条记录都挤进同一段，读起来结构全乱。这条测试盯的是**可读性**：
    /// 特殊字符不丢失，结构标记仍能区分记录。
    #[test]
    fn export_handles_special_content() {
        let store = Store::open_in_memory().expect("open");
        let id = store
            .create_session("openai", "m", Some("特殊内容测试"))
            .expect("create");
        store
            .add_segment(
                id,
                "mic",
                0,
                1000,
                "面试官问了：用 `pytest` 写一个测试",
                false,
                false,
            )
            .expect("seg");
        store
            .add_answer(
                id,
                Some(1),
                "多行答案第一行\n第二行含 **加粗** 与 | 管道\n第三行 `代码块` 结束",
                "m",
                AnswerMetrics::default(),
            )
            .expect("ans");
        store
            .add_segment(id, "mic", 1000, 2000, "第二段：表格示例", false, false)
            .expect("seg2");
        store
            .add_answer(
                id,
                Some(2),
                "| 列A | 列B |\n| --- | --- |\n| 1 | 2 |",
                "m",
                AnswerMetrics::default(),
            )
            .expect("ans2");

        let md = store.export_markdown(id).expect("export");

        // 内容不丢失：特殊字符原样保留
        assert!(md.contains("`pytest`"), "反引号内容应保留");
        assert!(md.contains("| 列A | 列B |"), "表格应保留");
        assert!(
            md.contains("第二行含 **加粗** 与 | 管道"),
            "加粗与管道应保留"
        );
        assert!(md.contains("多行答案第一行\n第二行"), "多行内容应保留");

        // 结构仍可读：每条记录的标记都在
        assert!(md.contains("# 面试记录 #"), "标题应存在");
        assert!(md.contains("## 转录与问答"), "转录小节应存在");
        assert!(md.contains("**面试官**"), "发言人标记应存在");
        assert!(md.contains("**回答**"), "答案标记应存在");
        // 第二条记录没有被第一条的多行内容吞掉
        assert!(
            md.find("第二段：表格示例").is_some(),
            "第二段转录应出现，不能被前面的多行答案吞掉"
        );
    }

    #[test]
    fn export_omits_empty_transcript_header() {
        let store = Store::open_in_memory().expect("open");

        // A：只有答案、没有转录
        let a = store.create_session("openai", "m", None).expect("create a");
        store
            .add_answer(a, None, "只有答案没有转录", "m", AnswerMetrics::default())
            .expect("ans");
        let text_a = store.export_markdown(a).expect("export a");
        assert!(
            !text_a.contains("## 转录与问答"),
            "没有转录时不该出现这个标题：{text_a}"
        );
        assert!(text_a.contains("## 其他回答"), "孤立答案仍要导出：{text_a}");
        assert!(text_a.contains("只有答案没有转录"), "{text_a}");

        // B：有转录、没答案 → 标题要在，但不该有空的「其他回答」
        let b = store.create_session("openai", "m", None).expect("create b");
        store
            .add_segment(b, "interviewer", 0, 1000, "面试官说了话", false, false)
            .expect("seg");
        let text_b = store.export_markdown(b).expect("export b");
        assert!(
            text_b.contains("## 转录与问答"),
            "有转录时标题要在：{text_b}"
        );
        assert!(
            !text_b.contains("## 其他回答"),
            "没有孤立答案不该出现该节：{text_b}"
        );
        assert!(text_b.contains("面试官说了话"), "{text_b}");
    }

    /// 数据健康摘要要能数出「结构完好但内容不对」的几类情况。
    ///
    /// `PRAGMA integrity_check` 只查文件结构；孤儿数据、未结束会话、空会话它
    /// 全都看不见。真机排障时这三类正是「记录不对劲」的常见来源。
    #[test]
    fn data_health_counts_orphans_and_unfinished() {
        let store = Store::open_in_memory().expect("open");

        // 一个正常会话（有转录 + 有答案 + 已结束）
        let ok = store
            .create_session("openai", "m", None)
            .expect("create ok");
        store
            .add_segment(ok, "mic", 0, 1000, "正常转录", false, false)
            .expect("seg");
        store
            .add_answer(ok, None, "正常答案", "m", AnswerMetrics::default())
            .expect("ans");
        store.end_session(ok).expect("end");

        // 一个未结束的会话（模拟进程被强杀）
        let _unfinished = store
            .create_session("openai", "m", None)
            .expect("create unf");

        // 一个空会话（开了就停）
        let empty = store
            .create_session("openai", "m", None)
            .expect("create empty");
        store.end_session(empty).expect("end empty");

        let h = store.data_health().expect("data_health");
        assert_eq!(h.sessions, 3, "应有 3 场会话");
        assert_eq!(h.segments, 1);
        assert_eq!(h.answers, 1);
        assert_eq!(h.unfinished, 1, "应有 1 场未结束");
        // 空会话 = 没有转录也没有答案，**与是否结束无关**。
        // 这里有两场：「未结束那个」和「开了就停那个」——两者都没内容。
        // （第一版断言写成 1，CI 直接红了：我把「未结束」和「有内容」混为一谈了。）
        assert_eq!(h.empty, 2, "未结束的和开了就停的那两场都没有内容");
        assert_eq!(h.orphan_segments, 0, "正常库不该有孤儿转录");
        assert_eq!(h.orphan_answers, 0, "正常库不该有孤儿答案");

        // 手工制造孤儿数据：直接插一条指向不存在会话的段落。
        //
        // 这里**必须临时关掉外键**——`Store` 的连接开了 `PRAGMA foreign_keys = ON`，
        // 正常插入会被 `FOREIGN KEY constraint failed`（extended_code 787）挡住。
        // 第一次写这条测试时没关，CI 直接红在那句上——而这恰恰反证了外键约束
        // 是生效的（正常路径确实不可能产生孤儿数据）。
        //
        // 关掉再插，模拟「外部工具改库 / 历史版本没开外键」这种真实来源。
        store
            .connection()
            .execute("PRAGMA foreign_keys = OFF", [])
            .expect("关外键");
        store
            .connection()
            .execute(
                "INSERT INTO segments (session_id, channel, start_ms, end_ms, text, is_echo, recovered)
                 VALUES (999999, 'mic', 0, 1, '孤儿', 0, 0)",
                [],
            )
            .expect("插孤儿");
        store
            .connection()
            .execute("PRAGMA foreign_keys = ON", [])
            .expect("开回外键");

        let h2 = store.data_health().expect("data_health 2");
        assert_eq!(h2.orphan_segments, 1, "应数出 1 段孤儿转录");
        assert_eq!(h2.segments, 2, "总段数要含孤儿");
    }

    /// 搜索必须覆盖备注（用户给会话起的名字）。
    ///
    /// 真机验证过的缺口：备注写进去之后详情页能读到，但搜「独特备注ZZTOP」
    /// 命中 0 条——而同一会话搜正文能命中（对照组）。SQL 里只 UNION 了
    /// `segments.text` 与 `answers.text`，没带 `sessions.note`。
    ///
    /// 为什么必须补：备注正是用户**最可能用来找回某场记录**的词
    /// （「腾讯一面」「字节二面」）。起了名却搜不到，比没起名更让人困惑——
    /// 他会以为那条记录丢了。
    #[test]
    fn search_covers_session_note() {
        let store = Store::open_in_memory().expect("open");
        let a = store.create_session("openai", "m", None).expect("create a");
        let b = store.create_session("openai", "m", None).expect("create b");

        assert!(
            store.set_session_note(a, "腾讯一面").expect("set note"),
            "备注应更新到记录"
        );

        // 备注命中
        assert_eq!(
            store.search_sessions("腾讯", 10).expect("search"),
            vec![a],
            "备注里的词必须能搜到"
        );
        assert_eq!(
            store.search_sessions("一面", 10).expect("search"),
            vec![a],
            "部分匹配也要命中（LIKE 语义）"
        );

        // 对照组：没设备注的会话不被这条路径带出来
        assert!(
            store
                .search_sessions("腾讯", 10)
                .expect("search")
                .iter()
                .all(|id| *id == a),
            "只有设了这条备注的会话才该命中"
        );
        let _ = b;

        // 清空备注后不再命中（空串存 NULL，不是空字符串）
        store.set_session_note(a, "").expect("clear note");
        assert!(
            store
                .search_sessions("腾讯", 10)
                .expect("search")
                .is_empty(),
            "备注清空后不该再命中"
        );
    }

    /// 搜索必须同时覆盖转录与答案。
    ///
    /// 只搜转录时，「当时 AI 给的那个方案」这类查找会落空，而界面上看不出
    /// 「搜过了但没搜答案」——用户只会以为没记过。
    #[test]
    fn search_covers_both_segments_and_answers() {
        let store = Store::open_in_memory().expect("open");
        let a = store.create_session("openai", "m", None).expect("create a");
        let b = store.create_session("openai", "m", None).expect("create b");

        // a 的关键词只在转录里
        store
            .add_segment(a, "interviewer", 0, 1000, "问过 kafka 吗", false, false)
            .expect("seg");
        // b 的关键词只在**答案**里
        store
            .add_answer(
                b,
                None,
                "我们用的是 playbook 方案",
                "m",
                AnswerMetrics::default(),
            )
            .expect("ans");

        // 转录命中
        assert_eq!(store.search_sessions("kafka", 10).expect("search"), vec![a]);
        // 答案命中——这是本轮扩的范围
        assert_eq!(
            store.search_sessions("playbook", 10).expect("search"),
            vec![b]
        );
        // 两个都不含的词
        assert!(
            store
                .search_sessions("zzz不存在", 10)
                .expect("search")
                .is_empty()
        );
        // 同一会话里转录与答案都命中时只返回一次（UNION 去重）
        store
            .add_answer(a, None, "kafka 的用法是这样", "m", AnswerMetrics::default())
            .expect("ans2");
        assert_eq!(store.search_sessions("kafka", 10).expect("search"), vec![a]);
    }

    /// 空会话识别：转录或答案任一存在就算有内容。
    ///
    /// 会话结束时会用它决定「这条记录该不该留」——判错两个方向后果不同：
    /// 误判有内容 → 历史里留下空壳（真机攒过 33 条）；
    /// 误判没内容 → **删掉用户真实的面试记录**（更严重）。
    #[test]
    fn session_has_content_detects_segments_and_answers() {
        let store = Store::open_in_memory().expect("open");
        let fresh = store.create_session("openai", "m", None).expect("create");
        // 刚建的空会话
        assert!(!store.session_has_content(fresh).expect("check"));

        // 只有转录
        let seg_only = store.create_session("openai", "m", None).expect("create");
        store
            .add_segment(seg_only, "interviewer", 0, 1000, "你好", false, false)
            .expect("seg");
        assert!(store.session_has_content(seg_only).expect("check"));

        // 只有答案
        let ans_only = store.create_session("openai", "m", None).expect("create");
        store
            .add_answer(ans_only, None, "答案", "m", AnswerMetrics::default())
            .expect("ans");
        assert!(store.session_has_content(ans_only).expect("check"));

        // 不存在的会话：EXISTS 两个子查询都查不到，返回 false 而不是报错
        assert!(!store.session_has_content(999_999).expect("check"));
    }

    /// 空会话的统计与删除必须用**同一个判据**。
    ///
    /// 两处判据不一致时会出现「数出来 33 条、删掉 30 条」——用户会以为有 3 条
    /// 删不掉，而实际是判据差了几个字节。
    #[test]
    fn empty_sessions_are_counted_and_deleted() {
        let store = Store::open_in_memory().expect("open");

        // 两条空会话
        store.create_session("openai", "m", None).expect("empty1");
        store.create_session("openai", "m", None).expect("empty2");
        // 一条有转录
        let with_seg = store.create_session("openai", "m", None).expect("seg");
        store
            .add_segment(with_seg, "interviewer", 0, 1000, "有内容", false, false)
            .expect("add seg");
        // 一条只有答案
        let with_ans = store.create_session("openai", "m", None).expect("ans");
        store
            .add_answer(with_ans, None, "有答案", "m", AnswerMetrics::default())
            .expect("add ans");

        assert_eq!(store.count_empty_sessions().expect("count"), 2);

        let removed = store.delete_empty_sessions().expect("delete");
        // 数的和删的必须相等
        assert_eq!(removed, 2);

        // 有内容的两个都还在
        assert_eq!(store.count_sessions().expect("total"), 2);
        assert!(store.session_has_content(with_seg).expect("check"));
        assert!(store.session_has_content(with_ans).expect("check"));

        // 再清理一次：没有空会话了
        assert_eq!(store.count_empty_sessions().expect("count"), 0);
        assert_eq!(store.delete_empty_sessions().expect("delete"), 0);
    }

    /// 备注：写入 / trim / 清除 / 不存在的 id。
    ///
    /// `note` 列是 `Option<String>`：空串必须存成 NULL 而不是 `Some("")`，
    /// 否则「没有备注」会有两种表示，列表标题的回落逻辑要先命中空串才落到时间戳。
    #[test]
    fn session_note_can_be_set_and_cleared() {
        let store = Store::open_in_memory().expect("open");
        let id = store.create_session("openai", "m", None).expect("create");
        // 初次写入
        assert!(store.set_session_note(id, "字节二面").expect("set"));
        assert_eq!(
            store.get_session(id).expect("get").note.as_deref(),
            Some("字节二面")
        );
        // 首尾空格会被 trim —— 列表标题不该带多余空白
        assert!(store.set_session_note(id, "  腾讯一面  ").expect("set"));
        assert_eq!(
            store.get_session(id).expect("get").note.as_deref(),
            Some("腾讯一面")
        );
        // 空串等于清除：存成 None，读侧只有一种「没有备注」
        assert!(store.set_session_note(id, "   ").expect("set"));
        assert_eq!(store.get_session(id).expect("get").note, None);
        // 不存在的 id 返回 false（调用方据此提示「会话已删除」）
        assert!(!store.set_session_note(id + 999, "x").expect("set"));
    }

    /// 构造一个内存库并开好会话。
    fn store_with_session() -> (Store, i64) {
        let store = Store::open_in_memory().expect("打开内存库");
        let session = store
            .create_session("openai", "deepseek-chat", Some("技术面"))
            .expect("创建会话");
        (store, session)
    }

    #[test]
    fn migration_runs_once_and_is_idempotent() {
        let store = Store::open_in_memory().expect("打开内存库");
        // 再跑一遍迁移不应报错，也不重复建表。
        migrate(&store.conn).expect("重复迁移");
        let version: i64 = store
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("读取版本");
        // 版本数必须等于迁移脚本个数（V2 加列后不再硬编码 1，
        // 以后加 V3 只改 MIGRATIONS 常量）。
        assert_eq!(version as usize, MIGRATIONS.len());
    }

    #[test]
    fn create_and_end_session_roundtrip() {
        let (store, id) = store_with_session();
        let session = store.get_session(id).expect("读取会话");
        assert_eq!(session.provider_id, "openai");
        assert_eq!(session.model_id, "deepseek-chat");
        assert_eq!(session.note.as_deref(), Some("技术面"));
        assert!(session.ended_at.is_none());
        assert!(session.started_at > 0);

        store.end_session(id).expect("结束会话");
        let ended = store.get_session(id).expect("再次读取");
        assert!(ended.ended_at.is_some());
        assert!(ended.ended_at >= Some(ended.started_at));
    }

    #[test]
    fn end_missing_session_reports_not_found() {
        let store = Store::open_in_memory().expect("打开内存库");
        match store.end_session(999) {
            Err(StoreError::SessionNotFound(id)) => assert_eq!(id, 999),
            other => panic!("应返回 SessionNotFound，实际：{other:?}"),
        }
        assert!(store.session_detail(999).is_err());
    }

    #[test]
    fn list_sessions_orders_and_limits() {
        let store = Store::open_in_memory().expect("打开内存库");
        let first = store.create_session("openai", "m1", None).expect("会话一");
        let second = store
            .create_session("anthropic", "m2", None)
            .expect("会话二");
        store
            .add_segment(first, CHANNEL_SYSTEM, 0, 100, "一", false, false)
            .expect("段落");
        store
            .add_answer(first, None, "答", "m1", AnswerMetrics::default())
            .expect("答案");

        let all = store.list_sessions(10).expect("列表");
        assert_eq!(all.len(), 2);
        // 同一毫秒创建时按 id 倒序，最后创建的在前。
        assert_eq!(all[0].session.id, second);
        assert_eq!(all[1].session.id, first);
        assert_eq!(all[1].segment_count, 1);
        assert_eq!(all[1].answer_count, 1);
        assert_eq!(all[0].segment_count, 0);

        let limited = store.list_sessions(1).expect("限制条数");
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].session.id, second);
    }

    #[test]
    fn segments_and_answers_roundtrip_in_detail() {
        let (store, session) = store_with_session();
        let q = store
            .add_segment(
                session,
                CHANNEL_SYSTEM,
                100,
                2_300,
                "你能说说这个项目的难点吗？",
                false,
                false,
            )
            .expect("提问段落");
        let echo = store
            .add_segment(
                session,
                CHANNEL_MIC,
                2_400,
                3_100,
                "你能说说这个项目",
                true,
                false,
            )
            .expect("回声段落");
        let a = store
            .add_segment(
                session,
                CHANNEL_MIC,
                3_200,
                9_000,
                "我主要负责自动化测试。",
                false,
                false,
            )
            .expect("回答段落");
        let _answer = store
            .add_answer(
                session,
                Some(q),
                "难点主要有三点：……",
                "deepseek-chat",
                AnswerMetrics {
                    ttft_ms: Some(420),
                    total_ms: Some(1_830),
                    token_in: Some(320),
                    token_out: Some(96),
                },
            )
            .expect("答案");

        let detail = store.session_detail(session).expect("详情");
        assert_eq!(detail.segments.len(), 3);
        assert_eq!(detail.segments[0].id, q);
        assert_eq!(detail.segments[1].id, echo);
        assert!(detail.segments[1].is_echo);
        assert!(!detail.segments[2].is_echo);
        assert_eq!(detail.segments[2].id, a);
        assert_eq!(detail.segments[0].speaker_label(), "提问");
        assert_eq!(detail.segments[2].speaker_label(), "我");

        assert_eq!(detail.answers.len(), 1);
        let answer = &detail.answers[0];
        assert_eq!(answer.question_segment_id, Some(q));
        assert_eq!(answer.ttft_ms, Some(420));
        assert_eq!(answer.total_ms, Some(1_830));
        assert_eq!(answer.token_in, Some(320));
        assert_eq!(answer.token_out, Some(96));
    }

    /// `recovered` 必须真的落库并读回：补录路径写 true、实时路径写 false，
    /// 复盘页「补录」标记全靠这一列（V2 迁移前该列不存在，标记是死 UI）。
    #[test]
    fn recovered_flag_roundtrips_and_defaults_to_zero() {
        let (store, session) = store_with_session();
        // 实时路径：false
        store
            .add_segment(session, CHANNEL_SYSTEM, 0, 500, "实时行", false, false)
            .expect("实时段落");
        // 补录路径：true
        store
            .add_segment(session, CHANNEL_SYSTEM, 600, 900, "补录行", false, true)
            .expect("补录段落");
        // 补录且回声：两个标记独立
        store
            .add_segment(session, CHANNEL_MIC, 1_000, 1_200, "补录回声", true, true)
            .expect("补录回声");

        let segments = store.segments_of(session).expect("段落列表");
        assert_eq!(segments.len(), 3);
        assert!(!segments[0].recovered, "实时行不应被标补录");
        assert!(segments[1].recovered, "补录行必须读回 true");
        assert!(
            segments[2].recovered && segments[2].is_echo,
            "两个标记互不影响"
        );

        // 迁移列有 DEFAULT 0：绕开 add_segment 直接插，模拟 V1 时代写下的老行。
        store
            .conn
            .execute(
                "INSERT INTO segments (session_id, channel, start_ms, end_ms, text, is_echo)
                 VALUES (?1, ?2, 0, 1, 'V1 老行', 0)",
                params![session, CHANNEL_SYSTEM],
            )
            .expect("插入 V1 风格行");
        let old = store.segments_of(session).expect("段落列表");
        let legacy = old
            .iter()
            .find(|segment| segment.text == "V1 老行")
            .expect("应能读回 V1 风格行");
        assert!(!legacy.recovered, "缺省列必须回落 false 而不是报错");
    }

    #[test]
    fn segments_order_by_start_ms_not_insert_order() {
        let (store, session) = store_with_session();
        store
            .add_segment(session, CHANNEL_SYSTEM, 500, 900, "后", false, false)
            .expect("段落一");
        store
            .add_segment(session, CHANNEL_SYSTEM, 100, 400, "先", false, false)
            .expect("段落二");
        let segments = store.segments_of(session).expect("段落列表");
        assert_eq!(segments[0].text, "先");
        assert_eq!(segments[1].text, "后");
    }

    #[test]
    fn delete_session_cascades_segments_and_answers() {
        let (store, session) = store_with_session();
        let q = store
            .add_segment(session, CHANNEL_SYSTEM, 0, 100, "问题", false, false)
            .expect("段落");
        store
            .add_answer(session, Some(q), "答案", "m", AnswerMetrics::default())
            .expect("答案");

        assert!(store.delete_session(session).expect("删除会话"));
        assert!(!store.delete_session(session).expect("再次删除"));
        assert!(store.get_session(session).is_err());

        let segments: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM segments", [], |row| row.get(0))
            .expect("统计段落");
        let answers: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM answers", [], |row| row.get(0))
            .expect("统计答案");
        assert_eq!(segments, 0);
        assert_eq!(answers, 0);
    }

    #[test]
    fn delete_all_sessions_clears_everything() {
        let store = Store::open_in_memory().expect("打开内存库");
        store.create_session("p", "m", None).expect("会话一");
        store.create_session("p", "m", None).expect("会话二");
        assert_eq!(store.delete_all_sessions().expect("全部删除"), 2);
        assert!(store.list_sessions(10).expect("列表").is_empty());
    }

    #[test]
    fn cleanup_removes_only_expired_sessions() {
        let store = Store::open_in_memory().expect("打开内存库");
        let old = store.create_session("p", "m", None).expect("旧会话");
        let recent = store.create_session("p", "m", None).expect("新会话");

        // 把旧会话的开始时间改到 40 天前。
        let long_ago = now_ms() - 40 * 24 * 60 * 60 * 1_000;
        store
            .conn
            .execute(
                "UPDATE sessions SET started_at = ?1 WHERE id = ?2",
                params![long_ago, old],
            )
            .expect("改时间");
        store
            .add_segment(old, CHANNEL_SYSTEM, 0, 100, "旧段落", false, false)
            .expect("旧段落");

        let deleted = store.cleanup_older_than_days(30).expect("清理");
        assert_eq!(deleted, 1);
        assert!(store.get_session(old).is_err());
        assert!(store.get_session(recent).is_ok());

        // 再清理一次没有可删的。
        assert_eq!(store.cleanup_older_than_days(30).expect("再次清理"), 0);
    }

    #[test]
    fn cleanup_with_zero_days_keeps_recent_sessions() {
        let store = Store::open_in_memory().expect("打开内存库");
        let session = store.create_session("p", "m", None).expect("会话");
        assert_eq!(store.cleanup_older_than_days(0).expect("清理"), 0);
        assert!(store.get_session(session).is_ok());
    }

    #[test]
    fn cleanup_cascades_segments_and_answers() {
        let store = Store::open_in_memory().expect("打开内存库");
        let old = store.create_session("p", "m", None).expect("旧会话");
        let q = store
            .add_segment(old, CHANNEL_SYSTEM, 0, 100, "旧问题", false, false)
            .expect("旧段落");
        store
            .add_answer(old, Some(q), "旧答案", "m", AnswerMetrics::default())
            .expect("旧答案");
        let long_ago = now_ms() - 40 * 24 * 60 * 60 * 1_000;
        store
            .conn
            .execute(
                "UPDATE sessions SET started_at = ?1 WHERE id = ?2",
                params![long_ago, old],
            )
            .expect("改时间");

        assert_eq!(store.cleanup_older_than_days(30).expect("清理"), 1);
        // 外键级联：不留下孤儿段落与答案。
        let segments: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM segments", [], |row| row.get(0))
            .expect("统计段落");
        let answers: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM answers", [], |row| row.get(0))
            .expect("统计答案");
        assert_eq!(segments, 0);
        assert_eq!(answers, 0);
    }

    #[test]
    fn delete_all_sessions_cascades_children() {
        let store = Store::open_in_memory().expect("打开内存库");
        let session = store.create_session("p", "m", None).expect("会话");
        let q = store
            .add_segment(session, CHANNEL_SYSTEM, 0, 100, "问题", false, false)
            .expect("段落");
        store
            .add_answer(session, Some(q), "答案", "m", AnswerMetrics::default())
            .expect("答案");

        assert_eq!(store.delete_all_sessions().expect("全部删除"), 1);
        let segments: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM segments", [], |row| row.get(0))
            .expect("统计段落");
        let answers: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM answers", [], |row| row.get(0))
            .expect("统计答案");
        assert_eq!(segments, 0);
        assert_eq!(answers, 0);
    }

    #[test]
    fn file_database_uses_wal_journal() {
        let path = std::env::temp_dir().join(format!(
            "hint-store-wal-test-{}-{}.db",
            std::process::id(),
            now_ms()
        ));
        {
            let store = Store::open(&path).expect("打开文件库");
            let mode: String = store
                .conn
                .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                .expect("读取 journal_mode");
            assert_eq!(mode.to_lowercase(), "wal");
            let foreign_keys: i64 = store
                .conn
                .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
                .expect("读取 foreign_keys");
            assert_eq!(foreign_keys, 1);
        }
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn markdown_export_keeps_every_regenerated_answer() {
        let (store, session) = store_with_session();
        let q = store
            .add_segment(
                session,
                CHANNEL_SYSTEM,
                0,
                100,
                "介绍一下自己",
                false,
                false,
            )
            .expect("提问");
        // 同一提问被「重新生成」两次，三条答案都应导出。
        store
            .add_answer(
                session,
                Some(q),
                "第一版回答",
                "m",
                AnswerMetrics::default(),
            )
            .expect("答案一");
        store
            .add_answer(
                session,
                Some(q),
                "第二版回答",
                "m",
                AnswerMetrics::default(),
            )
            .expect("答案二");

        let markdown = store.export_markdown(session).expect("导出");
        assert!(markdown.contains("第一版回答"));
        assert!(markdown.contains("第二版回答"));
        assert_eq!(markdown.matches("**回答**").count(), 2);
    }

    #[test]
    fn markdown_export_timeline_uses_session_start_offset() {
        let (store, session) = store_with_session();
        // 把开始时间固定到一个整点，避免依赖真实时钟。
        let started_at = 1_700_000_000_000i64; // 2023-11-14 22:13:20 UTC
        store
            .conn
            .execute(
                "UPDATE sessions SET started_at = ?1 WHERE id = ?2",
                params![started_at, session],
            )
            .expect("改时间");
        store
            .add_segment(session, CHANNEL_SYSTEM, 0, 100, "零秒", false, false)
            .expect("段落零");
        store
            .add_segment(
                session,
                CHANNEL_SYSTEM,
                65_000,
                66_000,
                "一分零五秒",
                false,
                false,
            )
            .expect("段落一");

        let markdown = store.export_markdown(session).expect("导出");
        let expected0 = format_clock(started_at);
        let expected1 = format_clock(started_at + 65_000);
        assert!(markdown.contains(&format!("**提问**（{expected0}）：零秒")));
        assert!(markdown.contains(&format!("**提问**（{expected1}）：一分零五秒")));
        // 时间轴必须递增，不能出现错位。
        assert!(markdown.find("零秒").unwrap() < markdown.find("一分零五秒").unwrap());
    }

    #[test]
    fn profiles_upsert_get_list_and_delete() {
        let store = Store::open_in_memory().expect("打开内存库");
        assert!(
            store
                .get_profile("resume", "default")
                .expect("读取")
                .is_none()
        );

        store
            .save_profile("resume", "default", "张三，五年测试开发")
            .expect("保存简历");
        store
            .save_profile("project", "hint", "面试提词器")
            .expect("保存项目一");
        store
            .save_profile("project", "atlas", "Playwright 测试平台")
            .expect("保存项目二");
        store
            .save_profile("resume", "default", "张三，六年测试开发")
            .expect("覆盖简历");

        let resume = store
            .get_profile("resume", "default")
            .expect("读取简历")
            .expect("简历存在");
        assert_eq!(resume.content, "张三，六年测试开发");
        assert!(resume.updated_at > 0);

        let projects = store.list_profiles("project").expect("项目列表");
        assert_eq!(projects.len(), 2);
        assert_eq!(store.list_profiles("resume").expect("简历列表").len(), 1);

        assert!(store.delete_profile("project", "hint").expect("删除项目"));
        assert!(!store.delete_profile("project", "hint").expect("再次删除"));
        assert_eq!(store.list_profiles("project").expect("项目列表").len(), 1);
    }

    #[test]
    fn markdown_export_contains_timeline_and_metrics() {
        let (store, session) = store_with_session();
        let q = store
            .add_segment(
                session,
                CHANNEL_SYSTEM,
                0,
                2_300,
                "你能说说这个项目的难点吗？",
                false,
                false,
            )
            .expect("提问");
        store
            .add_segment(
                session,
                CHANNEL_MIC,
                2_400,
                3_000,
                "你能说说这个项目",
                true,
                false,
            )
            .expect("回声");
        store
            .add_answer(
                session,
                Some(q),
                "难点主要有三点：第一，环境不稳定。",
                "deepseek-chat",
                AnswerMetrics {
                    ttft_ms: Some(400),
                    total_ms: Some(1_800),
                    token_in: Some(300),
                    token_out: Some(80),
                },
            )
            .expect("答案");
        store.end_session(session).expect("结束会话");

        let markdown = store.export_markdown(session).expect("导出");
        assert!(markdown.starts_with("# 面试记录 #"));
        assert!(markdown.contains("开始时间："));
        assert!(markdown.contains("模型：openai / deepseek-chat"));
        assert!(markdown.contains("备注：技术面"));
        assert!(markdown.contains("**提问**（"));
        assert!(markdown.contains("你能说说这个项目的难点吗？"));
        assert!(markdown.contains("（回声，已排除）"));
        assert!(markdown.contains("**回答**（首字 0.4s · 总耗时 1.8s · tokens 300/80）"));
        assert!(markdown.contains("难点主要有三点：第一，环境不稳定。"));
        assert!(!markdown.contains("其他回答"));
    }

    #[test]
    fn markdown_export_places_orphan_answers_last() {
        let (store, session) = store_with_session();
        store
            .add_segment(session, CHANNEL_SYSTEM, 0, 100, "介绍下自己", false, false)
            .expect("段落");
        store
            .add_answer(session, None, "我是张三。", "m", AnswerMetrics::default())
            .expect("游离答案");
        let markdown = store.export_markdown(session).expect("导出");
        assert!(markdown.contains("## 其他回答"));
        assert!(markdown.contains("我是张三。"));
    }

    #[test]
    fn markdown_export_handles_empty_session() {
        let (store, session) = store_with_session();
        let markdown = store.export_markdown(session).expect("导出");
        assert!(markdown.contains("没有转录内容"));
        assert!(!markdown.contains("## 转录与问答"));
    }

    #[test]
    fn markdown_export_missing_session_errors() {
        let store = Store::open_in_memory().expect("打开内存库");
        assert!(store.export_markdown(42).is_err());
    }

    #[test]
    fn segments_and_answers_are_scoped_per_session() {
        let store = Store::open_in_memory().expect("打开内存库");
        let first = store.create_session("p", "m", None).expect("会话一");
        let second = store.create_session("p", "m", None).expect("会话二");
        store
            .add_segment(first, CHANNEL_SYSTEM, 0, 100, "一", false, false)
            .expect("段落一");
        store
            .add_segment(second, CHANNEL_SYSTEM, 0, 100, "二", false, false)
            .expect("段落二");

        let detail = store.session_detail(first).expect("详情");
        assert_eq!(detail.segments.len(), 1);
        assert_eq!(detail.segments[0].text, "一");
        assert_eq!(detail.answers.len(), 0);
    }

    #[test]
    fn data_persists_across_reopen() {
        let path = std::env::temp_dir().join(format!(
            "hint-store-test-{}-{}.db",
            std::process::id(),
            now_ms()
        ));
        let session = {
            let store = Store::open(&path).expect("打开文件库");
            let session = store.create_session("openai", "m", None).expect("创建会话");
            store
                .add_segment(session, CHANNEL_SYSTEM, 0, 100, "持久化段落", false, false)
                .expect("段落");
            session
        };
        {
            let store = Store::open(&path).expect("重新打开");
            let detail = store.session_detail(session).expect("详情");
            assert_eq!(detail.segments.len(), 1);
            assert_eq!(detail.segments[0].text, "持久化段落");
        }
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn foreign_key_violation_is_reported() {
        let store = Store::open_in_memory().expect("打开内存库");
        assert!(
            store
                .add_segment(12345, CHANNEL_SYSTEM, 0, 100, "无主段落", false, false)
                .is_err()
        );
    }

    #[test]
    fn format_helpers_handle_out_of_range_timestamps() {
        assert_eq!(format_datetime(i64::MIN), i64::MIN.to_string());
        assert_eq!(format_clock(i64::MIN), i64::MIN.to_string());
        assert_eq!(format_seconds(1_830), "1.8s");
    }
}
