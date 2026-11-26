import { useCallback, useEffect, useRef, useState } from "react";
import { deriveStatusTone, statusToneClass } from "../settings/statusTone";
import type { SessionDetail, SessionSummary } from "./types";

type Props = {
  /** 当前要显示的会话（已按搜索过滤）。 */
  sessions: SessionSummary[];
  /** 会话总数，不受搜索影响：用于「清空全部（N）」，避免搜索时显示成过滤后的数量。 */
  totalCount: number;
  query: string;
  onQueryChange: (next: string) => void;
  searching: boolean;
  /** 是否处于搜索状态：区分「没搜」与「搜了没结果」。 */
  isSearching: boolean;
  /** 是否因为条数上限被截断。 */
  limited: boolean;
  loading: boolean;
  error: string;
  selectedId: string;
  detail: SessionDetail | null;
  detailLoading: boolean;
  detailError: string;
  notice: string;
  onRefresh: () => void;
  onOpen: (id: string) => void;
  onExport: () => void;
  onDelete: () => void;
  onDeleteAll: () => void;
  /** 批量导出全部会话到一个目录。 */
  onExportAll: () => void;
  /** 保存会话备注：空串表示清除，回落到时间戳。 */
  onRename: (id: string, note: string) => Promise<void>;
};

const SKELETON_KEYS = ["skeleton-1", "skeleton-2", "skeleton-3"];

export default function SessionReview(props: Props) {
  const {
    sessions,
    totalCount,
    query,
    onQueryChange,
    searching,
    isSearching,
    limited,
    loading,
    error,
    selectedId,
    detail,
    detailLoading,
    detailError,
    notice,
    onRefresh,
    onOpen,
    onExport,
    onDelete,
    onDeleteAll,
    onExportAll,
    onRename,
  } = props;

  return (
    <div className="review-layout">
      <section className="card review-list">
        <div className="card-head">
          <h2>历史会话</h2>
          <div className="action-row">
            <button type="button" className="ghost" onClick={onRefresh} disabled={loading}>
              {loading ? "加载中…" : "刷新"}
            </button>
            <button
              type="button"
              className="ghost"
              onClick={onExportAll}
              disabled={loading || totalCount === 0}
              title="把全部会话各导出一个 Markdown 文件到指定目录"
            >
              导出全部
            </button>
            <button
              type="button"
              className="danger small-danger"
              onClick={onDeleteAll}
              disabled={loading || totalCount === 0}
              title={
                totalCount > 0
                  ? `永久删除本机全部 ${totalCount} 条会话记录`
                  : "永久删除本机全部会话记录"
              }
            >
              {totalCount > 0 ? `清空全部（${totalCount}）` : "清空全部"}
            </button>
          </div>
        </div>

        {/* 按转录内容搜索：会话攒到几十条后，用户记得的是「问过 kafka 的那次」，
            不是「10 月 17 日 15:37 那次」。 */}
        <div className="session-search">
          <input
            type="search"
            className="form-input"
            value={query}
            onChange={(event) => onQueryChange(event.target.value)}
            placeholder="搜索会话内容（如某道题、某个技术词）"
            aria-label="搜索会话内容"
          />
          {searching && <span className="session-search-state">搜索中…</span>}
          {!searching && isSearching && (
            <span className="session-search-state">
              {sessions.length > 0
                ? `匹配 ${sessions.length} 条`
                : "没有匹配的会话"}
            </span>
          )}
        </div>

        {limited && (
          <p className="session-limit-hint">
            只显示最近 {sessions.length} 条（共 {totalCount} 条）。更早的记录仍在本机，
            导出全部可以拿到。
          </p>
        )}

        {error ? (
          <div className="card-body">
            <p className="error-text" role="alert">
              {error}
            </p>
          </div>
        ) : loading ? (
          <div className="card-body" aria-busy="true">
            <span className="visually-hidden">正在读取会话列表…</span>
            <ul className="session-items" aria-hidden="true">
              {SKELETON_KEYS.map((key) => (
                <li key={key}>
                  <div className="skeleton skeleton-text" />
                </li>
              ))}
            </ul>
          </div>
        ) : sessions.length === 0 ? (
          <div className="card-body">
            <EmptyState text="还没有会话记录。完成一次面试后，这里会列出可回看的转录与问答。" />
          </div>
        ) : (
          <div className="card-body">
            <ul className="session-items">
              {sessions.map((session) => {
                const title = session.note || session.startedAt || `会话 ${session.id}`;
                // meta 恒非空（模型缺失时已回落「模型未知」）；「时间未知」
                // 兜的其实是时长缺失——列表里没有时间列，时长才是第二段。
                const meta = [session.model || "模型未知", durationText(session) || "时间未知"].join(" · ");
                const active = session.id === selectedId;
                return (
                  <li key={session.id}>
                    <button
                      type="button"
                      className={`session-item${active ? " active" : ""}`}
                      onClick={() => onOpen(session.id)}
                      aria-current={active ? "true" : undefined}
                      title={`${title}${meta ? `（${meta}）` : ""}`}
                    >
                      <span className="session-title">{title}</span>
                      <span className="session-meta">{meta || "时间未知"}</span>
                    </button>
                  </li>
                );
              })}
            </ul>
          </div>
        )}
      </section>

      <section className="card review-detail">
        <div className="card-head">
          <h2>转录与问答</h2>
          <div className="action-row">
            <button
              type="button"
              onClick={onExport}
              disabled={!selectedId || detailLoading}
            >
              导出 Markdown
            </button>
            <button
              type="button"
              className="danger"
              onClick={onDelete}
              disabled={!selectedId || detailLoading}
            >
              删除
            </button>
          </div>
        </div>

        <div className="card-body" aria-busy={detailLoading || undefined}>
          {notice && (
            <p className={statusToneClass(deriveStatusTone(notice))} role="status">
              {notice}
            </p>
          )}
          {detailError && (
            <p className="error-text" role="alert">
              {detailError}
            </p>
          )}

          {!selectedId ? (
            <EmptyState text="从左侧选择一条会话查看详情。" />
          ) : detailLoading ? (
            <>
              <span className="visually-hidden">正在加载会话…</span>
              {SKELETON_KEYS.map((key) => (
                <div key={key} className="skeleton skeleton-text" />
              ))}
            </>
          ) : detail ? (
            <SessionBody detail={detail} onRename={onRename} />
          ) : (
            !detailError && <EmptyState text="没有可显示的内容。" />
          )}
        </div>
      </section>
    </div>
  );
}

function EmptyState({ text }: { text: string }) {
  return (
    <div className="empty-state">
      <svg
        className="empty-state-icon"
        width="36"
        height="36"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        aria-hidden="true"
      >
        <rect x="3" y="4" width="18" height="16" rx="2" />
        <path d="M7 9h10M7 13h10M7 17h6" />
      </svg>
      <p className="empty-state-text">{text}</p>
    </div>
  );
}

/**
 * 会话备注输入。
 *
 * 交互取舍：
 * - 失焦或按回车才提交，**不做逐字保存**——备注是偶尔改一次的东西，逐字写盘
 *   既浪费又会在打字中途产生一堆半截内容；
 * - 提交后与 props 里的 note 比较，没变就不调用后端；
 * - 保存失败把后端原因显示出来（而不是静默回滚——用户会以为存上了）。
 */
function SessionNoteInput({
  id,
  note,
  onRename,
}: {
  id: string;
  note: string;
  onRename: (id: string, note: string) => Promise<void>;
}) {
  const [draft, setDraft] = useState(note);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  /**
   * 「这次 blur 是 Esc 触发的，别保存」。
   *
   * 必须在 blur 之前置位、在 commit 里消费：`onKeyDown` 里调 `blur()` 会同步触发
   * `onBlur → commit`，而 commit 拿到的是**闭包里的旧 draft**（setDraft 还没生效）。
   * 只写 setDraft(note) + blur() 会让 Esc 变成「保存我刚才想放弃的内容」——
   * 这条是测试抓出来的，不是推演。
   */
  const cancelNextBlur = useRef(false);

  // 切换会话时同步草稿：否则会把上一条的备注文本留在框里。
  useEffect(() => {
    setDraft(note);
    setError("");
  }, [id, note]);

  const commit = useCallback(async () => {
    if (cancelNextBlur.current) {
      cancelNextBlur.current = false;
      setDraft(note);
      return;
    }
    const next = draft.trim();
    if (next === note.trim()) return;
    setSaving(true);
    setError("");
    try {
      await onRename(id, next);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  }, [draft, note, id, onRename]);

  return (
    <div className="session-note">
      <input
        type="text"
        className="form-input session-note-input"
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={() => void commit()}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            event.currentTarget.blur();
          }
          if (event.key === "Escape") {
            // 置位后 blur：commit 会消费这个标志跳过保存（见 cancelNextBlur 注释）。
            cancelNextBlur.current = true;
            event.currentTarget.blur();
          }
        }}
        placeholder="给这次面试起个名字（如「字节二面」）"
        aria-label="会话备注"
        disabled={saving}
      />
      {saving && <span className="session-note-state">保存中…</span>}
      {error && (
        <span className="session-note-state error-text" role="alert">
          {error}
        </span>
      )}
    </div>
  );
}

function SessionBody({
  detail,
  onRename,
}: {
  detail: SessionDetail;
  onRename: (id: string, note: string) => Promise<void>;
}) {
  return (
    <div className="review-body">
      {/* 备注是列表标题的首选显示名（note > startedAt > id）：没有它，41 条会话在
          界面上全是时间戳，想标「字节二面」做不到。 */}
      <SessionNoteInput
        id={detail.summary.id}
        note={detail.summary.note}
        onRename={onRename}
      />
      <p className="muted small">
        {detail.summary.startedAt || "开始时间未知"}
        {detail.summary.endedAt ? ` → ${detail.summary.endedAt}` : ""}
        {detail.summary.model ? ` · ${detail.summary.model}` : ""}
        {durationText(detail.summary) ? ` · 时长 ${durationText(detail.summary)}` : ""}
      </p>

      <h3 className="review-subtitle">转录</h3>
      {detail.segments.length === 0 ? (
        <p className="muted small">该会话没有转录记录。</p>
      ) : (
        <>
          <div className="legend" aria-label="转录颜色图例">
            <span className="legend-item">
              <i className="legend-swatch swatch-interviewer" aria-hidden="true" />
              提问
            </span>
            <span className="legend-item">
              <i className="legend-swatch swatch-me" aria-hidden="true" />
              我
            </span>
            <span className="legend-item">
              <i className="legend-swatch swatch-echo" aria-hidden="true" />
              回声
            </span>
            <span className="legend-item">
              <i className="legend-swatch swatch-recovered" aria-hidden="true" />
              补录
            </span>
          </div>
          <div className="review-transcript">
            {detail.segments.map((segment) => (
              <div
                key={segment.id}
                className={`line ${segment.channel === "interviewer" ? "interviewer" : "me"}${segment.isEcho ? " echo" : ""}${segment.recovered ? " recovered" : ""}`}
              >
                <span className="who">{segment.channel === "interviewer" ? "提问" : "我"}</span>
                <span className="what">
                  {segment.text}
                  {segment.isEcho && <em className="echo-tag">回声</em>}
                  {segment.recovered && <em className="recovered-tag">补录</em>}
                </span>
              </div>
            ))}
          </div>
        </>
      )}

      <h3 className="review-subtitle">回答</h3>
      {detail.answers.length === 0 ? (
        <p className="muted small">该会话没有生成过答案。</p>
      ) : (
        <div className="review-answers">
          {detail.answers.map((answer) => (
            <div key={answer.id} className="review-answer">
              <pre className="answer-box">{answer.text || "（空答案）"}</pre>
              <p className="muted small">
                {answer.model || "模型未知"}
                {answer.ttftMs !== null ? ` · 首字 ${(answer.ttftMs / 1000).toFixed(1)}s` : ""}
              </p>
            </div>
          ))}
        </div>
      )}

      {(detail.segments.length === 0 && detail.answers.length === 0) && (
        <p className="muted small">
          该会话内容为空。若后端已接入存储，请确认会话在结束面试时已写入。
        </p>
      )}
    </div>
  );
}

/** 由 startedAt / endedAt 文本推算时长；后端格式为「YYYY-MM-DD HH:MM:SS」。 */
function durationText(summary: { startedAt: string; endedAt: string }): string {
  const start = parseTime(summary.startedAt);
  const end = parseTime(summary.endedAt);
  if (start === null || end === null || end < start) return "";
  const totalSeconds = Math.round((end - start) / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) return `${hours} 小时 ${minutes} 分`;
  if (minutes > 0) return `${minutes} 分 ${seconds} 秒`;
  return `${seconds} 秒`;
}

function parseTime(text: string): number | null {
  if (!text) return null;
  const parsed = Date.parse(text.replace(" ", "T"));
  return Number.isFinite(parsed) ? parsed : null;
}