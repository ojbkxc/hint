//! 面试会话运行时：采集 → 分段 → 本地 STT → 存储 → LLM 流式回答。
//!
//! 线程与锁模型：
//! - 采集回调（`hint-mic` / `hint-system-audio` 线程）只做分段与入队，不阻塞；
//! - 专用 `hint-stt-dispatch` 线程持有 [`WorkerClient`]，串行做推理与落库；
//! - LLM 流式在 Tauri 的异步运行时里执行，回调里只做 emit；
//! - 共享状态全部是短临界区的 `Mutex`，任何锁都不跨 `await`。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hint_audio::{
    AudioChunk, AudioConfig, AudioEngine, Channel, JointChannel, JointSpeechState,
    JointSpeechTransition, Levels,
};
use hint_core::audio::SPEECH_RMS;
use hint_core::llm::{self, ChatRequest, Turn, TurnRole};
use hint_core::pipeline;
use hint_core::secret;
use hint_core::settings::{Settings, TriggerMode};
use hint_core::session::{LifecycleAction, LifecycleDir, LifecycleState};
use hint_core::stt::{
    is_covered_by_live, split_into_speech_clips, ClipOptions, Outage, SttGapTracker,
};
use hint_store::{AnswerMetrics, CHANNEL_MIC, CHANNEL_SYSTEM};
use hint_stt::WorkerClient;
use hint_stt::download::user_message_for;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use crate::{AppState, lock};

/// 事件名（与前端 `HINT_EVENTS` 一致）。
pub(crate) const EVT_TRANSCRIPT: &str = "hint://transcript";
pub(crate) const EVT_ANSWER_DELTA: &str = "hint://answer-delta";
pub(crate) const EVT_ANSWER_DONE: &str = "hint://answer-done";
pub(crate) const EVT_LEVELS: &str = "hint://levels";
pub(crate) const EVT_STATUS: &str = "hint://status";
/// 与面试会话无关的提示（模型下载、热键注册、截图预留），不覆盖会话状态。
pub(crate) const EVT_NOTICE: &str = "hint://notice";
/// 排队问题的实时数量（入队/出队/清空时发），供浮层刷新「排队 N」。
pub(crate) const EVT_QUEUE: &str = "hint://queue";
/// 浮层不透明度变化（设置页保存后实时生效）。
pub(crate) const EVT_OVERLAY_OPACITY: &str = "hint://overlay-opacity";
pub(crate) const EVT_MODEL_PROGRESS: &str = "hint://model-progress";
/// 双通道联合语音状态迁移（谁开始/停止说话）。
pub(crate) const EVT_SPEECH_EDGE: &str = "hint://speech-edge";

const TARGET_RATE: u32 = 16_000;
/// 噪声门阈值（软膝门，仅压低底噪；可在 AudioConfig 里关闭做 A/B）。
const NOISE_GATE_THRESHOLD: f32 = 0.003;
/// 静音挂起时长，决定断句。
const SILENCE_HANGOVER_MS: u64 = 650;
/// 有效语音最短时长；更短的判为噪声丢弃。
const MIN_SPEECH_MS: u64 = 250;
/// 语音起点前回补的音频，避免吞掉首字。
const PREROLL_MS: u64 = 300;
/// 单段最长时长；超过即强制断句，避免长段推理延迟过高。
const MAX_SEGMENT_MS: u64 = 12_000;
/// 面试官近文本参与回声比较的窗口。
const RECENT_INTERVIEWER_MS: u64 = 8_000;
/// 历史轮次保留上限（提示词组装还会再截最后 6 条）。
const HISTORY_LIMIT: usize = 24;
/// STT worker 空闲回收时长。
const WORKER_IDLE: Duration = Duration::from_secs(300);
/// STT 待处理队列容量；满了直接丢新段，不阻塞音频回调线程。
const STT_QUEUE_CAPACITY: usize = 32;
/// STT 断线恢复：每路环形缓冲容量（毫秒）。
const RING_CAP_MS: u64 = 120_000;
/// 出错时 outage 回溯时长（覆盖已发送但丢失的音频）。
const ERROR_PREROLL_MS: u64 = 3_000;
/// 看门狗：无识别结果的时长门槛。
const STALL_MS: u64 = 15_000;
/// 看门狗：判 stall 所需的最低语音输入。
const STALL_SPEECH_MS: u64 = 4_000;
/// stall 退避上限。
const MAX_STALL_BACKOFF_MS: u64 = 120_000;
/// outage 关闭后等待补录的时长。
const SETTLE_MS: u64 = 2_000;

/// 发送事件；失败只记日志，绝不影响主流程。
pub(crate) fn emit<S: serde::Serialize + Clone>(app: &AppHandle, event: &str, payload: S) {
    if let Err(error) = app.emit(event, payload) {
        tracing::warn!(event, %error, "emit 失败");
    }
}

/// 推送一条状态事件（不带 payload 之外的信息）。
pub(crate) fn emit_status(app: &AppHandle, state: &str, message: impl Into<String>) {
    emit(
        app,
        EVT_STATUS,
        json!({ "state": state, "message": message.into() }),
    );
}

/// 推送一条「与面试会话无关」的提示（模型下载、热键注册、截图预留等）。
///
/// 这些提示不能走 [`emit_status`]：那会覆盖会话状态（`listening` → `idle`），
/// 让前端把「面试进行中」的按钮状态画错。这里改用独立事件，前端只当作提示展示。
pub(crate) fn emit_notice(app: &AppHandle, level: &str, message: impl Into<String>) {
    emit(
        app,
        EVT_NOTICE,
        json!({ "level": level, "message": message.into() }),
    );
}

// ---------------------------------------------------------------------------
// 分段
// ---------------------------------------------------------------------------

/// 一段已经切好的语音（16k 单声道 f32，含首字前的回补）。
struct RawSegment {
    samples: Vec<f32>,
    start_ms: i64,
    end_ms: i64,
}

fn samples_to_ms(len: usize) -> u64 {
    (len as u64 * 1_000) / u64::from(TARGET_RATE)
}

/// 基于 RMS 门限的简易分段器：预卷 + 静音挂起 + 最短/最长语音约束。
struct Segmenter {
    preroll: Vec<f32>,
    max_preroll: usize,
    samples: Vec<f32>,
    speaking: bool,
    speech_ms: u64,
    silence_ms: u64,
    total_ms: i64,
}

impl Segmenter {
    fn new() -> Self {
        Self {
            preroll: Vec::new(),
            max_preroll: (PREROLL_MS as usize * TARGET_RATE as usize) / 1_000,
            samples: Vec::new(),
            speaking: false,
            speech_ms: 0,
            silence_ms: 0,
            total_ms: 0,
        }
    }

    /// 追加一块音频并切出完整段；返回本块的 RMS，供断线看门狗复用，避免重复计算。
    fn push(&mut self, chunk: &[f32], out: &mut Vec<RawSegment>) -> f32 {
        let chunk_ms = samples_to_ms(chunk.len());
        let rms = hint_core::audio::audio_metrics(chunk).0;
        let loud = rms >= SPEECH_RMS;

        if !self.speaking {
            self.preroll.extend_from_slice(chunk);
            let overflow = self.preroll.len().saturating_sub(self.max_preroll);
            if overflow > 0 {
                self.preroll.drain(..overflow);
            }
            if loud {
                self.speaking = true;
                self.samples = std::mem::take(&mut self.preroll);
                self.speech_ms = chunk_ms;
                self.silence_ms = 0;
            }
        } else {
            self.samples.extend_from_slice(chunk);
            if loud {
                self.speech_ms += chunk_ms;
                self.silence_ms = 0;
            } else {
                self.silence_ms += chunk_ms;
            }
        }

        self.total_ms += chunk_ms as i64;

        if !self.speaking {
            return rms;
        }
        let buffered_ms = samples_to_ms(self.samples.len());
        let too_long = buffered_ms >= MAX_SEGMENT_MS;
        if self.silence_ms >= SILENCE_HANGOVER_MS || too_long {
            if self.speech_ms >= MIN_SPEECH_MS || too_long {
                let samples = std::mem::take(&mut self.samples);
                out.push(RawSegment {
                    end_ms: self.total_ms,
                    start_ms: self.total_ms - samples_to_ms(samples.len()) as i64,
                    samples,
                });
            } else {
                self.samples.clear();
            }
            self.speaking = false;
            self.speech_ms = 0;
            self.silence_ms = 0;
            self.preroll.clear();
        }
        rms
    }
}

/// 编一个 44 字节标准头的 16k 单声道 PCM16 WAV（hint-stt worker 只接受这种形态）。
fn encode_wav(samples: &[f32]) -> Vec<u8> {
    let data_bytes = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&TARGET_RATE.to_le_bytes());
    out.extend_from_slice(&(TARGET_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for &sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * 32_767.0).round() as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

pub(crate) struct SttJob {
    channel: Channel,
    wav: Vec<u8>,
    start_ms: i64,
    end_ms: i64,
}

// ---------------------------------------------------------------------------
// 共享状态
// ---------------------------------------------------------------------------

struct RecentLine {
    at: Instant,
    text: String,
}

/// 答案队列上限：连续多问时最多排 5 个，超出丢弃新问题并提示。
const ANSWER_QUEUE_LIMIT: usize = 5;

/// 归一化文本用于去重：只保留字母/数字（含中文），转小写。
///
/// STT 对同一句话可能给出「你、好。」「你好」两种形态，标点与空白不应该
/// 让重复问题绕过去重。
fn normalize_for_dedup(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// 判断两次识别是否同一句。
///
/// 只承认「归一化后完全相同」或「去掉礼貌/语气前后缀后相同」。
/// 不能用「短句被长句包含且长度接近」——那会把真实追问
/// （「介绍一下你的项目」→「介绍一下你的项目难点」）当成重复静默吞掉。
fn similar_text(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return true;
    }
    trim_politeness(a) == trim_politeness(b)
}

/// 去掉常见的礼貌前缀与语气尾缀，仅用于判重。
///
/// 列表刻意保持很短：多剥一个字就可能把不同问题合并。
fn trim_politeness(text: &str) -> &str {
    const LEADING: [&str; 9] = [
        "那么",
        "那",
        "然后",
        "麻烦你",
        "麻烦",
        "请你",
        "请",
        "想问一下",
        "我想问一下",
    ];
    const TRAILING: [&str; 8] = ["呢", "吗", "呀", "啊", "吧", "哈", "好不好", "可以吗"];

    let mut rest = text;
    // 前缀只剥一次（「那请你…」这种多层组合极少，多剥反而危险）。
    if let Some(prefix) = LEADING
        .iter()
        .filter(|prefix| rest.starts_with(*prefix))
        .max_by_key(|prefix| prefix.chars().count())
    {
        rest = &rest[prefix.len()..];
    }
    for suffix in TRAILING {
        if let Some(stripped) = rest.strip_suffix(suffix) {
            rest = stripped;
            break;
        }
    }
    rest
}

/// 排队中的一个问题。
#[derive(Debug, Clone, PartialEq, Eq)]
struct QueuedQuestion {
    text: String,
    segment_id: Option<i64>,
    is_regenerate: bool,
}

/// 入队结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnqueueOutcome {
    Queued,
    Duplicate,
    Full,
}

/// 问题队列 + worker 状态，同一把锁保护以避免丢唤醒。
///
/// 纯结构、不依赖 Tauri，便于单测。
#[derive(Default)]
struct QuestionQueue {
    items: VecDeque<QueuedQuestion>,
    /// 是否已有 worker 在消费；用于保证同一时刻只有一个流式请求。
    worker_running: bool,
    /// 正在作答的问题（归一化后），用于与「队尾」一起去重。
    current: Option<String>,
}

impl QuestionQueue {
    fn enqueue(&mut self, question: QueuedQuestion) -> EnqueueOutcome {
        let normalized = normalize_for_dedup(&question.text);
        if normalized.is_empty() {
            return EnqueueOutcome::Duplicate;
        }
        if let Some(current) = self.current.as_deref()
            && similar_text(&normalized, current)
        {
            return EnqueueOutcome::Duplicate;
        }
        if let Some(tail) = self.items.back()
            && similar_text(&normalized, &normalize_for_dedup(&tail.text))
        {
            return EnqueueOutcome::Duplicate;
        }
        if self.items.len() >= ANSWER_QUEUE_LIMIT {
            return EnqueueOutcome::Full;
        }
        self.items.push_back(question);
        EnqueueOutcome::Queued
    }

    /// 队列非空且当前没有 worker 时才返回 true，并置位「worker 在跑」。
    fn begin_worker(&mut self) -> bool {
        if self.worker_running || self.items.is_empty() {
            return false;
        }
        self.worker_running = true;
        true
    }

    fn take_next(&mut self) -> Option<QueuedQuestion> {
        let question = self.items.pop_front()?;
        self.current = Some(normalize_for_dedup(&question.text));
        Some(question)
    }

    fn finish_current(&mut self) {
        self.current = None;
    }

    /// worker 退出时调用；返回 true 表示退出瞬间又有新问题入队，需要重启。
    fn end_worker(&mut self) -> bool {
        self.worker_running = false;
        !self.items.is_empty()
    }

    fn pending(&self) -> usize {
        self.items.len()
    }

    /// 停止会话 / 重新生成时清空排队与当前问题（不动 worker 标志）。
    fn clear(&mut self) {
        self.items.clear();
        self.current = None;
    }
}

/// 提示词上下文：历史轮次 + 最近面试官文本 + 最近一个问题。
struct AnswerCtx {
    history: Vec<Turn>,
    recent_interviewer: VecDeque<RecentLine>,
    last_question: Option<String>,
}

/// STT 断线恢复状态：两路 tracker + 待补录队列。
struct SttRecoveryState {
    interviewer: SttGapTracker,
    me: SttGapTracker,
    queue: VecDeque<PendingRecovery>,
}

/// 一个待补录的断线窗口。
struct PendingRecovery {
    channel: Channel,
    outage: Outage,
    ready_at: Instant,
    /// 已补录到的时间点（音频时间轴毫秒）。补录被实时任务打断后从这里继续，
    /// 避免重复识别同一片段。
    resume_from_ms: i64,
}

/// 一次会话内跨线程共享的状态。
pub(crate) struct SessionShared {
    session_id: i64,
    settings: Settings,
    trigger: TriggerMode,
    cancelled: AtomicBool,
    answer_cancel: Mutex<Option<Arc<AtomicBool>>>,
    /// 连续多问的答案队列；worker 串行消费，不再取消上一题。
    answers: Mutex<QuestionQueue>,
    ctx: Mutex<AnswerCtx>,
    /// STT 断线恢复状态（环形缓冲 + 看门狗 + 补录队列）。
    recovery: Mutex<SttRecoveryState>,
}

impl SessionShared {
    fn push_turn(&self, role: TurnRole, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let mut ctx = lock(&self.ctx);
        // 连续两个「我」轮如果内容近似（生成答案被用户念出来、STT 重复），
        // 用新文本替换旧轮，避免历史里堆重复答案；不影响面试官轮次。
        if role == TurnRole::Me
            && let Some(last) = ctx.history.last_mut()
            && last.role == TurnRole::Me
            && similar_text(&normalize_for_dedup(&last.text), &normalize_for_dedup(text))
        {
            last.text = text.to_string();
            return;
        }
        ctx.history.push(Turn {
            role,
            text: text.to_string(),
        });
        let overflow = ctx.history.len().saturating_sub(HISTORY_LIMIT);
        if overflow > 0 {
            ctx.history.drain(..overflow);
        }
    }

    /// 把模型生成的答案写入历史，供下一题组装提示词时使用。
    fn push_generated_answer(&self, text: &str) {
        self.push_turn(TurnRole::Me, text);
    }

    fn record_interviewer(&self, text: &str, is_question: bool) {
        let mut ctx = lock(&self.ctx);
        let now = Instant::now();
        ctx.recent_interviewer.push_back(RecentLine {
            at: now,
            text: text.to_string(),
        });
        while let Some(front) = ctx.recent_interviewer.front() {
            if now.duration_since(front.at) > Duration::from_millis(RECENT_INTERVIEWER_MS)
                || ctx.recent_interviewer.len() > 16
            {
                ctx.recent_interviewer.pop_front();
            } else {
                break;
            }
        }
        if is_question {
            ctx.last_question = Some(text.to_string());
        }
    }

    fn recent_interviewer_texts(&self) -> Vec<String> {
        let ctx = lock(&self.ctx);
        // 取用时也要按时间过滤：面试官长时间没说话时，队列里保留的旧文本
        // 不该继续参与回声判定，否则候选人的正常回答会被误判成回声。
        let now = Instant::now();
        ctx.recent_interviewer
            .iter()
            .filter(|line| {
                now.saturating_duration_since(line.at)
                    <= Duration::from_millis(RECENT_INTERVIEWER_MS)
            })
            .map(|line| line.text.clone())
            .collect()
    }

    fn history_snapshot(&self) -> Vec<Turn> {
        lock(&self.ctx).history.clone()
    }

    fn last_question(&self) -> Option<String> {
        lock(&self.ctx).last_question.clone()
    }
}

/// 两个通道各自的预卷/分段状态。
struct Segmenters {
    interviewer: Segmenter,
    me: Segmenter,
}

// ---------------------------------------------------------------------------
// 公开操作（命令、托盘、热键共用）
// ---------------------------------------------------------------------------

/// 当前是否有会话在运行。
pub(crate) fn is_running(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    lock(&state.session).is_some()
}

/// 会话生命周期请求的唯一入口：锁内决策，锁外执行。
///
/// 所有 start/stop/toggle（命令、托盘、热键）都经此串行化，从结构上消除
/// check-exec 竞态。返回 `(决策结果, 本次执行结果)`；未执行（合并/排队/
/// 已达成）时结果为 `Ok(0)`，由调用方按语义处理。
pub(crate) fn submit_lifecycle(
    app: &AppHandle,
    dir: LifecycleDir,
) -> (LifecycleAction, Result<i64, String>) {
    let action = begin_decision(app, |queue| queue.request(dir));
    execute_lifecycle(app, action)
}

/// 切换语义：方向在锁内根据生命周期状态决定，命令层与热键共用。
///
/// 返回 `(决定的方向, 决策结果, 本次执行结果)`。启动中再按一次会排队停止，
/// 停止中再按一次会排队重新启动。
pub(crate) fn submit_toggle(
    app: &AppHandle,
) -> (LifecycleDir, LifecycleAction, Result<i64, String>) {
    let mut chosen = LifecycleDir::Start;
    let action = begin_decision(app, |queue| {
        let (dir, action) = queue.request_toggle();
        chosen = dir;
        action
    });
    let (_, result) = execute_lifecycle(app, action);
    (chosen, action, result)
}

/// 在锁内完成「决策 + begin」。
///
/// 两步必须在同一把锁内：否则两个线程同时拿到 `Run`，第二个 start 会因
/// 「会话已在进行中」失败并把状态回写成 Idle，之后 Stop 被判成 Noop，
/// 会话再也停不下来。
fn begin_decision(
    app: &AppHandle,
    decide: impl FnOnce(&mut hint_core::session::LifecycleQueue) -> LifecycleAction,
) -> LifecycleAction {
    let state = app.state::<AppState>();
    let mut queue = lock(&state.lifecycle);
    let action = decide(&mut queue);
    if let LifecycleAction::Run(d) = action {
        queue.begin(d);
    }
    action
}

fn execute_lifecycle(
    app: &AppHandle,
    action: LifecycleAction,
) -> (LifecycleAction, Result<i64, String>) {
    match action {
        LifecycleAction::Run(d) => (action, run_lifecycle(app, d)),
        _ => (action, Ok(0)),
    }
}

/// 会话是否「忙」：已有运行中的会话，或正在启动/停止。
///
/// 启动过程中 `state.session` 还是 `None`（模型冷加载要 10–30 秒），
/// 只查 `is_running` 会让删模型、清空记录这类操作插进启动流程。
pub(crate) fn is_busy(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    if lock(&state.session).is_some() {
        return true;
    }
    lock(&state.lifecycle).state() != LifecycleState::Idle
}

/// 执行一个已经 `begin` 的生命周期动作：执行 → finish → drain 排队请求。
fn run_lifecycle(app: &AppHandle, dir: LifecycleDir) -> Result<i64, String> {
    let result = match dir {
        LifecycleDir::Start => start_session(app),
        LifecycleDir::Stop => stop_session(app).map(|_| 0),
    };
    // 失败时按「实际是否还有会话」回写，而不是一律回退：
    // - Start 失败但会话其实在跑（重复启动）→ 记 Active；
    // - Stop 失败但会话已经没了（并发收尾）→ 记 Idle。
    // 否则队列状态会和真实会话长期不一致，之后的热键切换全部失效。
    let running = is_running(app);
    let finished_ok = result.is_ok() || running == (dir == LifecycleDir::Start);
    {
        let state = app.state::<AppState>();
        lock(&state.lifecycle).finish(dir, finished_ok);
    }
    drain_lifecycle(app);
    result
}

/// 处理排队中的异向请求（在途请求已结束，队列里有未达成的项）。
fn drain_lifecycle(app: &AppHandle) {
    loop {
        let next = {
            let state = app.state::<AppState>();
            let mut queue = lock(&state.lifecycle);
            if queue.in_flight() {
                return;
            }
            match queue.next_pending() {
                Some(dir) => {
                    queue.begin(dir);
                    dir
                }
                None => return,
            }
        };
        // 排队请求的执行结果无需返回给调用方，只需完成。
        let _ = run_lifecycle(app, next);
    }
}

fn current_shared(app: &AppHandle) -> Option<Arc<SessionShared>> {
    let state = app.state::<AppState>();
    lock(&state.session)
        .as_ref()
        .map(|runtime| Arc::clone(&runtime.shared))
}

/// 启动会话：建库记录 → 冷启 worker → 启动双通道采集 → 起分发线程。
pub(crate) fn start_session(app: &AppHandle) -> Result<i64, String> {
    {
        let state = app.state::<AppState>();
        if lock(&state.session).is_some() {
            return Err("会话已在进行中，请先结束当前会话。".to_string());
        }
    }

    let state = app.state::<AppState>();
    let settings = Settings::load(&state.settings_path);
    let provider = settings.provider.as_str().to_string();

    if !secret::has(&provider) {
        let message = format!(
            "未配置 {} 的 API Key，请先在设置页保存后再开始会话。",
            provider.to_uppercase()
        );
        emit_status(app, "error", &message);
        return Err(message);
    }
    if !state.model.is_ready() {
        let message =
            "本地语音模型尚未下载（约 240MB），请先在设置页下载模型后再开始会话。".to_string();
        emit_status(app, "error", &message);
        return Err(message);
    }

    emit_status(app, "connecting", "正在创建会话…");

    let session_id = {
        let store = lock(&state.store);
        store
            .create_session(&provider, &settings.model, None)
            .map_err(|error| format!("创建会话记录失败：{error}"))?
    };

    let config = AudioConfig {
        mic_device: Some(settings.mic_device.clone()).filter(|value| !value.is_empty()),
        output_device: Some(settings.output_device.clone()).filter(|value| !value.is_empty()),
        noise_gate_threshold: Some(NOISE_GATE_THRESHOLD),
        ..AudioConfig::default()
    };

    let exe = std::env::current_exe().map_err(|error| {
        cleanup_failed_start(app, session_id);
        format!("无法定位程序路径，语音 worker 无法启动：{error}")
    })?;
    // worker 冷启要加载约 240MB 模型（最长 60 秒），期间必须给出阶段提示，
    // 否则 UI 会一直停在「正在创建会话…」。
    emit_status(
        app,
        "connecting",
        "正在加载本地语音模型，首次启动约需 10–30 秒…",
    );
    let worker_config = state.model.worker_config();
    let client = match WorkerClient::spawn(&exe, worker_config) {
        Ok(client) => client,
        Err(error) => {
            cleanup_failed_start(app, session_id);
            let message = format!("本地语音识别启动失败：{}", user_message_for(&error));
            emit_status(app, "error", &message);
            return Err(message);
        }
    };
    // 音频回调线程也持有句柄：看门狗触发时直接 abort 正在进行（可能已挂死）的推理。
    // `WorkerClient::abort` 只写一个原子标志、不抢 state 锁，可以在采集线程上安全调用。
    let client = Arc::new(client);

    let shared = Arc::new(SessionShared {
        session_id,
        trigger: settings.trigger_mode,
        settings,
        cancelled: AtomicBool::new(false),
        answer_cancel: Mutex::new(None),
        answers: Mutex::new(QuestionQueue::default()),
        ctx: Mutex::new(AnswerCtx {
            history: Vec::new(),
            recent_interviewer: VecDeque::new(),
            last_question: None,
        }),
        recovery: Mutex::new(SttRecoveryState {
            interviewer: SttGapTracker::new(JointChannel::Interviewer, RING_CAP_MS),
            me: SttGapTracker::new(JointChannel::Me, RING_CAP_MS),
            queue: VecDeque::new(),
        }),
    });

    let (jobs_tx, jobs_rx) = sync_channel::<SttJob>(STT_QUEUE_CAPACITY);
    let drop_warned = Arc::new(AtomicBool::new(false));
    let segmenters = Arc::new(Mutex::new(Segmenters {
        interviewer: Segmenter::new(),
        me: Segmenter::new(),
    }));

    let on_chunk = {
        let shared = Arc::clone(&shared);
        let segmenters = Arc::clone(&segmenters);
        let jobs = jobs_tx.clone();
        let drop_warned = Arc::clone(&drop_warned);
        let client = Arc::clone(&client);
        move |chunk: AudioChunk| {
            if shared.cancelled.load(Ordering::Acquire) {
                return;
            }
            let mut finished = Vec::new();
            let (start_ms, end_ms, rms) = {
                let mut guard = lock(&segmenters);
                let segmenter = match chunk.channel {
                    Channel::Interviewer => &mut guard.interviewer,
                    Channel::Me => &mut guard.me,
                };
                let before = segmenter.total_ms;
                let rms = segmenter.push(&chunk.samples, &mut finished);
                (before, segmenter.total_ms, rms)
            };
            // 断线恢复：喂环形缓冲 + 看门狗。
            let dur_ms = (end_ms - start_ms).max(0) as u64;
            let stalled = {
                let mut recovery = lock(&shared.recovery);
                let tracker = match chunk.channel {
                    Channel::Interviewer => &mut recovery.interviewer,
                    Channel::Me => &mut recovery.me,
                };
                tracker.on_audio(
                    &chunk.samples,
                    start_ms,
                    dur_ms,
                    rms,
                    SPEECH_RMS,
                    STALL_MS,
                    STALL_SPEECH_MS,
                    MAX_STALL_BACKOFF_MS,
                )
            };
            if stalled {
                // 直接在这里中止推理：dispatcher 此刻可能正阻塞在 transcribe 里，
                // 由它自己检查标志永远等不到。abort 只写原子标志，采集线程可安全调用。
                tracing::warn!("语音识别停滞，中止当前推理并回收 worker 触发冷启");
                client.abort();
            }
            for segment in finished {
                let job = SttJob {
                    channel: chunk.channel,
                    wav: encode_wav(&segment.samples),
                    start_ms: segment.start_ms,
                    end_ms: segment.end_ms,
                };
                match jobs.try_send(job) {
                    Ok(()) => {}
                    Err(std::sync::mpsc::TrySendError::Full(_)) => {
                        // 推理跟不上时丢新段，不阻塞采集线程；只告警一次。
                        if !drop_warned.swap(true, Ordering::AcqRel) {
                            tracing::warn!("语音识别队列已满，丢弃新段（推理速度跟不上采集）");
                        }
                    }
                    Err(std::sync::mpsc::TrySendError::Disconnected(_)) => break,
                }
            }
        }
    };

    let on_levels = {
        let app = app.clone();
        let shared = Arc::clone(&shared);
        move |levels: Levels| {
            if shared.cancelled.load(Ordering::Acquire) {
                return;
            }
            emit(
                &app,
                EVT_LEVELS,
                json!({ "mic": clamp01(levels.mic_rms), "system": clamp01(levels.system_rms) }),
            );
        }
    };

    let on_error = {
        let app = app.clone();
        move |message: String| {
            tracing::warn!(%message, "音频引擎错误");
            if message.starts_with("警告：") {
                emit_status(&app, "listening", message);
                return;
            }
            let fatal = is_fatal_audio_error(&message);
            emit_status(
                &app,
                if fatal { "error" } else { "listening" },
                format!("音频采集中断：{message}"),
            );
            if fatal {
                end_session_after_audio_failure(&app);
            }
        }
    };

    let on_speech_edge = {
        let app = app.clone();
        let shared = Arc::clone(&shared);
        move |transition: JointSpeechTransition| {
            if shared.cancelled.load(Ordering::Acquire) {
                return;
            }
            emit(
                &app,
                EVT_SPEECH_EDGE,
                json!({
                    "channel": channel_key(transition.channel),
                    "speaking": transition.speaking,
                    "joint": joint_key(transition.joint),
                    "atMs": transition.at_ms,
                    "msSinceOtherEdge": (transition.ms_since_other_edge != u64::MAX)
                        .then_some(transition.ms_since_other_edge),
                }),
            );
        }
    };

    let handle = match AudioEngine::start(config, on_chunk, on_levels, on_error, on_speech_edge) {
        Ok(handle) => handle,
        Err(error) => {
            cleanup_failed_start(app, session_id);
            let message = format!("音频采集启动失败：{error}");
            emit_status(app, "error", &message);
            return Err(message);
        }
    };
    // 启动阶段发生致命错误时 on_error 已发出、引擎已停；此时不能把会话登记为运行中。
    if !handle.is_running() {
        handle.stop();
        cleanup_failed_start(app, session_id);
        let message = "音频采集启动后立即停止，请检查音频设备与隐私权限后重试。".to_string();
        emit_status(app, "error", &message);
        return Err(message);
    }

    let dispatch_done = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    let dispatcher = {
        let thread_app = app.clone();
        let shared = Arc::clone(&shared);
        let done = Arc::clone(&dispatch_done);
        std::thread::Builder::new()
            .name("hint-stt-dispatch".to_string())
            .spawn(move || dispatcher_loop(thread_app, shared, jobs_rx, client, done))
            .map_err(|error| {
                cleanup_failed_start(app, session_id);
                format!("无法启动语音分发线程：{error}")
            })?
    };

    {
        let mut guard = lock(&state.session);
        if guard.is_some() {
            // 竞态：另一个 start 抢先成功，回收本次资源（含刚建的空会话记录）。
            shared.cancelled.store(true, Ordering::SeqCst);
            handle.stop();
            drop(jobs_tx);
            cleanup_failed_start(app, session_id);
            return Err("会话已在进行中。".to_string());
        }
        *guard = Some(crate::RunningSession {
            shared,
            handle,
            jobs: Some(jobs_tx),
            dispatcher: Some(dispatcher),
        });
        *lock(&state.dispatch_done) = Some(Arc::clone(&dispatch_done));
    }

    crate::set_session_running(app, true);
    emit_status(app, "listening", "面试进行中");
    tracing::info!(session_id, "session started");
    Ok(session_id)
}

fn cleanup_failed_start(app: &AppHandle, session_id: i64) {
    let state = app.state::<AppState>();
    let store = lock(&state.store);
    if let Err(error) = store.delete_session(session_id) {
        tracing::warn!(%error, session_id, "清理失败的会话记录出错");
    }
}

fn is_fatal_audio_error(message: &str) -> bool {
    // hint-audio 在 rt.fatal() 之前上报的文案（两处通道 + 线程 panic 兜底）。
    message.contains("采集引擎停止") || message.contains("引擎停止") || message.contains("异常退出")
}

/// 音频通道致命退出后，在后台把会话收尾（不能在采集线程里直接 stop）。
///
/// 走生命周期队列而不是直调 `stop_session`：否则队列里的状态会停在 Active，
/// 之后的热键/托盘「切换」会一直判成 Stop，会话再也起不来。
fn end_session_after_audio_failure(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let task_app = app.clone();
        match tauri::async_runtime::spawn_blocking(move || {
            submit_lifecycle(&task_app, LifecycleDir::Stop)
        })
        .await
        {
            Ok((_, Ok(_))) => {
                emit_status(&app, "error", "音频采集中断，会话已结束");
            }
            Ok((_, Err(error))) => tracing::warn!(%error, "音频中断后结束会话失败"),
            Err(error) => tracing::warn!(%error, "音频中断后结束会话任务失败"),
        }
    });
}

fn clamp01(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// 联合状态机的通道名（与前端约定的小写）。
fn channel_key(channel: JointChannel) -> &'static str {
    match channel {
        JointChannel::Interviewer => "interviewer",
        JointChannel::Me => "me",
    }
}

/// 联合状态名（与前端约定的小写）。
fn joint_key(joint: JointSpeechState) -> &'static str {
    match joint {
        JointSpeechState::Neither => "neither",
        JointSpeechState::InterviewerSpeaking => "interviewer",
        JointSpeechState::MeSpeaking => "me",
        JointSpeechState::Both => "both",
    }
}

/// 结束会话：取消流 → 停采集 → 关分发线程（回收 worker）→ 落库结束时间。
pub(crate) fn stop_session(app: &AppHandle) -> Result<(), String> {
    let runtime = {
        let state = app.state::<AppState>();
        lock(&state.session).take()
    };
    let Some(runtime) = runtime else {
        return Err("当前没有进行中的会话。".to_string());
    };

    let crate::RunningSession {
        shared,
        handle,
        jobs,
        dispatcher,
    } = runtime;

    shared.cancelled.store(true, Ordering::SeqCst);
    {
        // 会话结束：清空断线恢复状态，不再跑补录推理。
        let mut recovery = lock(&shared.recovery);
        recovery.interviewer.reset();
        recovery.me.reset();
        recovery.queue.clear();
    }
    if let Some(flag) = lock(&shared.answer_cancel).take() {
        flag.store(true, Ordering::SeqCst);
    }
    // 清空排队的问题；worker 看到 cancelled 后会自行退出。
    lock(&shared.answers).clear();
    emit_queue_len(app, &shared);
    // 先停采集（join 两个采集线程），再释放句柄让回调闭包连同 Sender 一起
    // 析构；分发线程随即看到 channel 断开并退出，不必阻塞等待推理。
    handle.stop();
    drop(handle);
    drop(jobs);

    {
        let state = app.state::<AppState>();
        let store = lock(&state.store);
        if let Err(error) = store.end_session(shared.session_id) {
            tracing::warn!(%error, "结束会话记录失败");
        }
    }

    if let Some(join) = dispatcher {
        // 推理最长 120s，后台回收，避免卡住 UI / 退出流程。
        std::thread::spawn(move || {
            if join.join().is_err() {
                tracing::warn!("语音分发线程退出时发生 panic");
            }
        });
    }

    emit(
        app,
        EVT_ANSWER_DONE,
        json!({ "text": "", "model": "", "ttftMs": null }),
    );
    crate::set_session_running(app, false);
    emit(app, EVT_LEVELS, json!({ "mic": 0.0, "system": 0.0 }));
    emit_status(app, "idle", "会话已结束");
    tracing::info!(session_id = shared.session_id, "session stopped");
    Ok(())
}

/// 退出前同步回收会话（尽力而为，阻塞上限由 STT 超时控制）。
pub(crate) fn shutdown(app: &AppHandle) {
    if is_running(app) && stop_session(app).is_err() {
        tracing::warn!("退出时结束会话失败");
    }
}

/// 解析「要回答的问题」。
///
/// 优先用启发式判定出的 `last_question`；manual 模式下启发式可能一直不更新，
/// 此时回退到最近一条面试官文本（先看 8 秒窗口，再看完整历史），
/// 让用户手动按 Ctrl+Shift+A 时不依赖自动判定。
fn resolve_question(shared: &SessionShared) -> Option<String> {
    if let Some(question) = shared.last_question() {
        let trimmed = question.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    let from_recent = shared
        .recent_interviewer_texts()
        .into_iter()
        .rev()
        .find(|text| !text.trim().is_empty());
    if let Some(text) = from_recent {
        return Some(text.trim().to_string());
    }
    shared
        .history_snapshot()
        .into_iter()
        .rev()
        .find(|turn| turn.role == TurnRole::Interviewer && !turn.text.trim().is_empty())
        .map(|turn| turn.text.trim().to_string())
}

/// 重新生成最近一个问题的回答：取消正在生成的流、清空排队，把最新问题
/// 以 `is_regenerate` 重新放到队首（等价于「现在就要最新问题的答案」）。
pub(crate) fn regenerate(app: &AppHandle) -> Result<(), String> {
    let shared = current_shared(app)
        .ok_or_else(|| "还没有开始面试会话，请先点「开始面试」或按 Ctrl+Shift+R。".to_string())?;
    let question = resolve_question(&shared).ok_or_else(|| {
        "还没有识别到面试官的问题，等面试官提问后再按 Ctrl+Shift+A（或点浮层「重新生成」）。"
            .to_string()
    })?;
    // 取消当前流（失败/取消不会发错误提示），并丢掉积压的问题。
    if let Some(flag) = lock(&shared.answer_cancel).as_ref() {
        flag.store(true, Ordering::SeqCst);
    }
    lock(&shared.answers).clear();
    enqueue_question(app, &shared, question, None, true);
    Ok(())
}

/// 入队一个问题；worker 没在跑时拉起 worker。
fn enqueue_question(
    app: &AppHandle,
    shared: &Arc<SessionShared>,
    text: String,
    segment_id: Option<i64>,
    is_regenerate: bool,
) {
    let outcome = lock(&shared.answers).enqueue(QueuedQuestion {
        text,
        segment_id,
        is_regenerate,
    });
    match outcome {
        EnqueueOutcome::Queued => {}
        EnqueueOutcome::Duplicate => {
            tracing::debug!("重复问题，跳过入队");
            return;
        }
        EnqueueOutcome::Full => {
            // 队列满时丢弃新问题：面试里最新问题通常更重要，但连续丢会伤体验，
            // 这里明确提示用户可手动作答。
            emit_notice(
                app,
                "info",
                "问题积压较多，已忽略最新一条；可按 Ctrl+Shift+A 优先作答当前问题",
            );
            return;
        }
    }
    emit_queue_len(app, shared);
    start_answer_worker(app, shared);
}

/// 把当前排队数发给前端；流式过程中新增问题时浮层也能立刻看到「排队 N」。
fn emit_queue_len(app: &AppHandle, shared: &Arc<SessionShared>) {
    let pending = lock(&shared.answers).pending();
    emit(app, EVT_QUEUE, json!({ "pending": pending }));
}

/// 拉起唯一的答案 worker。已有 worker 时直接返回，由它消费新入队的问题。
fn start_answer_worker(app: &AppHandle, shared: &Arc<SessionShared>) {
    if !lock(&shared.answers).begin_worker() {
        return;
    }
    let app = app.clone();
    let shared = Arc::clone(shared);
    tauri::async_runtime::spawn(async move {
        loop {
            if shared.cancelled.load(Ordering::Acquire) {
                lock(&shared.answers).clear();
                break;
            }
            let Some(job) = lock(&shared.answers).take_next() else {
                break;
            };
            emit_queue_len(&app, &shared);
            answer_one(&app, &shared, job).await;
            lock(&shared.answers).finish_current();
        }
        // 退出瞬间可能刚好有新问题入队（end_worker 与入队同一把锁），
        // 返回 true 时重新拉起，避免丢唤醒。
        let restart = lock(&shared.answers).end_worker();
        if restart && !shared.cancelled.load(Ordering::Acquire) {
            start_answer_worker(&app, &shared);
        }
    });
}

// 托盘 / 热键用的异步入口：内部全是阻塞操作，必须离开事件循环线程。
// 经生命周期队列串行化，避免与命令层并发时出现 check-exec 竞态。
pub(crate) fn spawn_start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let task_app = app.clone();
        match tauri::async_runtime::spawn_blocking(move || {
            submit_lifecycle(&task_app, LifecycleDir::Start)
        })
        .await
        {
            Ok((_, result)) => {
                if let Err(error) = result {
                    tracing::warn!(%error, "托盘/热键启动会话失败");
                    // 会话已经在跑时（重复按热键）不能发 status=error：那会把前端
                    // 的「结束面试」按钮禁掉。只有确实没有会话时才走状态错误。
                    if is_running(&app) {
                        emit_notice(&app, "error", error);
                    } else {
                        emit_status(&app, "error", error);
                    }
                }
            }
            Err(error) => {
                tracing::warn!(%error, "托盘/热键启动会话任务失败");
                emit_status(&app, "error", format!("启动任务失败：{error}"));
            }
        }
    });
}

pub(crate) fn spawn_stop(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let task_app = app.clone();
        match tauri::async_runtime::spawn_blocking(move || {
            submit_lifecycle(&task_app, LifecycleDir::Stop)
        })
        .await
        {
            Ok((_, result)) => {
                if let Err(error) = result {
                    tracing::warn!(%error, "托盘/热键结束会话失败");
                    // 结束失败通常意味着会话其实还在跑，不能把状态改成 error。
                    emit_notice(&app, "error", error);
                }
            }
            Err(error) => {
                tracing::warn!(%error, "托盘/热键结束会话任务失败");
                emit_notice(&app, "error", format!("停止任务失败：{error}"));
            }
        }
    });
}

/// 托盘 / 热键的「切换」入口：方向在生命周期队列锁内决定。
pub(crate) fn spawn_toggle(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let task_app = app.clone();
        match tauri::async_runtime::spawn_blocking(move || {
            let (_, _, result) = submit_toggle(&task_app);
            result
        })
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                tracing::warn!(%error, "托盘/热键切换会话失败");
                // 启动失败且确实没有会话时走状态错误，否则只提示，避免把
                // 正在进行的会话标成「启动失败」。
                if is_running(&app) {
                    emit_notice(&app, "error", error);
                } else {
                    emit_status(&app, "error", error);
                }
            }
            Err(error) => {
                tracing::warn!(%error, "托盘/热键切换会话任务失败");
                emit_status(&app, "error", format!("切换任务失败：{error}"));
            }
        }
    });
}

pub(crate) fn spawn_regenerate(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = regenerate(&app) {
            // 这里绝不能发 status=error：会话还在正常跑，前端会把「结束面试」
            // 按钮禁用、显示「启动失败」。用 notice 单独提示。
            emit_notice(&app, "error", error);
        }
    });
}

// ---------------------------------------------------------------------------
// STT 分发
// ---------------------------------------------------------------------------

fn dispatcher_loop(
    app: AppHandle,
    shared: Arc<SessionShared>,
    jobs: Receiver<SttJob>,
    client: Arc<WorkerClient>,
    done: Arc<(Mutex<bool>, std::sync::Condvar)>,
) {
    // 补录期间收到的实时任务暂存在这里，下个循环优先处理。
    let mut parked: Option<SttJob> = None;
    loop {
        if shared.cancelled.load(Ordering::Acquire) {
            break;
        }
        // 实时任务优先：补录期间被暂存的任务先消化掉。
        if let Some(job) = parked.take() {
            process_job(&app, &shared, &client, job);
            continue;
        }
        match jobs.recv_timeout(Duration::from_secs(1)) {
            Ok(job) => process_job(&app, &shared, &client, job),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if client.idle_for().is_some_and(|idle| idle > WORKER_IDLE) {
                    tracing::info!("空闲超过 5 分钟，回收语音 worker");
                    client.stop();
                }
                // 队列空闲（1 秒无新 job）时才执行断线补录；补录途中来了实时段会暂停。
                run_recovery(&app, &shared, &client, &jobs, &mut parked);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    client.stop();
    // 通知等待方（remove_model）：worker 进程已回收。
    let (done_lock, done_cv) = &*done;
    *lock(done_lock) = true;
    done_cv.notify_all();
}

/// 处理一个实时识别任务。
///
/// 时间戳统一用音频时间轴（`job.start_ms` / `job.end_ms`），与断线恢复的
/// 环形缓冲、outage 窗口保持一致。不要改用 dispatcher 自己的 `Instant`：
/// 两者起点不同（音频从首块开始计时、dispatcher 从线程启动计时），
/// 混用会让补录窗口整体错位。
fn process_job(app: &AppHandle, shared: &Arc<SessionShared>, client: &WorkerClient, job: SttJob) {
    let result = client.transcribe(&job.wav, "auto");
    match result {
        Ok(transcript) => {
            let text = transcript.text.trim().to_string();
            // 识别成功：关闭 outage（若有）并入补录队列。
            let outage = {
                let mut recovery = lock(&shared.recovery);
                let tracker = match job.channel {
                    Channel::Interviewer => &mut recovery.interviewer,
                    Channel::Me => &mut recovery.me,
                };
                tracker.on_transcript(&text, job.end_ms)
            };
            if let Some(outage) = outage {
                let resume_from_ms = outage.start_ms;
                let mut recovery = lock(&shared.recovery);
                recovery.queue.push_back(PendingRecovery {
                    channel: job.channel,
                    outage,
                    ready_at: Instant::now() + Duration::from_millis(SETTLE_MS),
                    resume_from_ms,
                });
            }
            if !text.is_empty() {
                handle_transcript(app, shared, job.channel, &text, job.start_ms, job.end_ms);
            }
        }
        Err(error) => {
            tracing::warn!(%error, "本地语音识别失败");
            {
                let mut recovery = lock(&shared.recovery);
                let tracker = match job.channel {
                    Channel::Interviewer => &mut recovery.interviewer,
                    Channel::Me => &mut recovery.me,
                };
                // outage 必须覆盖失败任务本身的音频：单个分段最长 12 秒，
                // 只回溯 3 秒会让段首大部分内容永远补不回来。
                let failed_ms = (job.end_ms - job.start_ms).max(0) as u64;
                tracker.on_error(job.end_ms, failed_ms + ERROR_PREROLL_MS);
            }
            if !shared.cancelled.load(Ordering::Acquire) {
                emit_status(
                    app,
                    "listening",
                    format!("语音识别失败，稍后自动重试：{}", user_message_for(&error)),
                );
            }
        }
    }
}

/// 执行一个到期的补录窗口：切语音片段 → 识别 → 去重 → 写库 + 上报。
///
/// 补录途中若来了实时任务，会暂存到 `parked` 并退回队列，下个空闲周期
/// 从 `resume_from_ms` 继续——实时转录永远优先。
fn run_recovery(
    app: &AppHandle,
    shared: &Arc<SessionShared>,
    client: &WorkerClient,
    jobs: &Receiver<SttJob>,
    parked: &mut Option<SttJob>,
) {
    let pending = {
        let mut recovery = lock(&shared.recovery);
        let now = Instant::now();
        let position = recovery
            .queue
            .iter()
            .position(|item| item.ready_at <= now);
        position.and_then(|index| recovery.queue.remove(index))
    };
    let Some(mut pending) = pending else {
        return;
    };
    let (frames, live_texts) = {
        let recovery = lock(&shared.recovery);
        let tracker = match pending.channel {
            Channel::Interviewer => &recovery.interviewer,
            Channel::Me => &recovery.me,
        };
        let start = pending.resume_from_ms.max(pending.outage.start_ms);
        let end = pending.outage.recovered_at_ms.unwrap_or(start);
        let frames = tracker.ring().range(start, end);
        // 只比较窗口附近的实时文本：把很久之后的文本也算进来会误判重复。
        let live = tracker.live_texts_between(start - 5_000, end + 5_000);
        (frames, live)
    };
    if frames.is_empty() {
        return;
    }
    let clips = split_into_speech_clips(&frames, &ClipOptions::default());
    let state = app.state::<AppState>();
    let mut recovered = 0usize;
    for clip in clips {
        if shared.cancelled.load(Ordering::Acquire) {
            break;
        }
        // 实时任务优先：补录途中来了新段就暂停，下个空闲周期从断点继续。
        if let Ok(job) = jobs.try_recv() {
            *parked = Some(job);
            let mut recovery = lock(&shared.recovery);
            recovery.queue.push_back(PendingRecovery {
                channel: pending.channel,
                outage: pending.outage.clone(),
                ready_at: Instant::now() + Duration::from_millis(SETTLE_MS),
                resume_from_ms: clip.start_ms,
            });
            tracing::debug!("补录暂停：实时语音段优先");
            return;
        }
        let wav = encode_wav(&clip.samples);
        match client.transcribe(&wav, "auto") {
            Ok(transcript) => {
                let text = transcript.text.trim().to_string();
                if text.is_empty() {
                    pending.resume_from_ms = clip.end_ms;
                    continue;
                }
                if is_covered_by_live(&text, &live_texts, 0.7) {
                    pending.resume_from_ms = clip.end_ms;
                    continue;
                }
                let is_echo = if pending.channel == Channel::Me {
                    let recent = shared.recent_interviewer_texts();
                    let refs: Vec<&str> = recent.iter().map(String::as_str).collect();
                    pipeline::is_echo(&text, &refs)
                } else {
                    false
                };
                let channel_id = if pending.channel == Channel::Interviewer {
                    CHANNEL_SYSTEM
                } else {
                    CHANNEL_MIC
                };
                {
                    let store = lock(&state.store);
                    if let Err(error) = store.add_segment(
                        shared.session_id,
                        channel_id,
                        clip.start_ms,
                        clip.end_ms,
                        &text,
                        is_echo,
                    ) {
                        tracing::warn!(%error, "补录写段落失败");
                    }
                }
                emit(
                    app,
                    EVT_TRANSCRIPT,
                    json!({
                        "channel": if pending.channel == Channel::Interviewer { "interviewer" } else { "me" },
                        "text": text,
                        "isEcho": is_echo,
                        "recovered": true,
                    }),
                );
                // 补录到历史供后续问答参考，但不重新入队作答，避免重复答案。
                if !is_echo {
                    match pending.channel {
                        Channel::Interviewer => {
                            shared.push_turn(TurnRole::Interviewer, &text);
                            shared.record_interviewer(&text, false);
                        }
                        Channel::Me => shared.push_turn(TurnRole::Me, &text),
                    }
                }
                recovered += 1;
            }
            Err(error) => {
                tracing::warn!(%error, "补录识别失败");
            }
        }
        // 无论成功失败都推进游标，避免同一片段反复重试卡死补录。
        pending.resume_from_ms = clip.end_ms;
    }
    if recovered > 0 {
        tracing::info!(recovered, "断线补录完成");
    }
}

fn handle_transcript(
    app: &AppHandle,
    shared: &Arc<SessionShared>,
    channel: Channel,
    text: &str,
    start_ms: i64,
    end_ms: i64,
) {
    if shared.cancelled.load(Ordering::Acquire) {
        return;
    }
    let state = app.state::<AppState>();

    match channel {
        Channel::Interviewer => {
            let segment_id = {
                let store = lock(&state.store);
                store
                    .add_segment(
                        shared.session_id,
                        CHANNEL_SYSTEM,
                        start_ms,
                        end_ms,
                        text,
                        false,
                    )
                    .map_err(|error| tracing::warn!(%error, "写面试官段落失败"))
                    .ok()
            };
            emit(
                app,
                EVT_TRANSCRIPT,
                json!({ "channel": "interviewer", "text": text, "isEcho": false }),
            );

            let is_question = pipeline::looks_like_question(text);
            shared.push_turn(TurnRole::Interviewer, text);
            shared.record_interviewer(text, is_question);

            if is_question && pipeline::should_generate(shared.trigger, true) {
                enqueue_question(app, shared, text.to_string(), segment_id, false);
            }
        }
        Channel::Me => {
            let recent = shared.recent_interviewer_texts();
            let refs: Vec<&str> = recent.iter().map(String::as_str).collect();
            let is_echo = pipeline::is_echo(text, &refs);

            {
                let store = lock(&state.store);
                if let Err(error) = store.add_segment(
                    shared.session_id,
                    CHANNEL_MIC,
                    start_ms,
                    end_ms,
                    text,
                    is_echo,
                ) {
                    tracing::warn!(%error, "写麦克风段落失败");
                }
            }
            emit(
                app,
                EVT_TRANSCRIPT,
                json!({ "channel": "me", "text": text, "isEcho": is_echo }),
            );

            if !is_echo {
                shared.push_turn(TurnRole::Me, text);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// LLM 流式生成
// ---------------------------------------------------------------------------

/// 作答一个问题（由 worker 串行调用）。
///
/// 上下文在**此刻**才快照：这样能包含前面已经完成并写入历史的答案。
async fn answer_one(app: &AppHandle, shared: &Arc<SessionShared>, job: QueuedQuestion) {
    if shared.cancelled.load(Ordering::Acquire) {
        return;
    }
    let question = job.text;
    let mut history = shared.history_snapshot();
    // 当前问题在检测时已被 push_turn 写入；排队期间后续问题也会写进历史。
    // 作答时要把「当前问题及其之后」的面试官轮全部截掉，否则 Q1 的 prompt
    // 里会提前出现 Q2，模型可能一次回答两问或答错题。
    if let Some(position) = history
        .iter()
        .rposition(|turn| turn.role == TurnRole::Interviewer && turn.text.trim() == question.trim())
    {
        // 丢掉当前问题以及排队期间已写入的后续面试官发言。
        history.truncate(position);
    }
    let pending = lock(&shared.answers).pending();
    let settings = shared.settings.clone();
    let request = ChatRequest {
        system: pipeline::build_system_prompt(&settings),
        user: pipeline::build_user_prompt(&question, &history),
        ..ChatRequest::default()
    };

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = lock(&shared.answer_cancel);
        if let Some(previous) = slot.replace(Arc::clone(&cancel)) {
            previous.store(true, Ordering::SeqCst);
        }
    }
    // 停止会话发生在上面的检查之后：此时新 cancel 标志尚未被 stop_session
    // 看到，必须自己确认一次，否则会在会话结束后继续流式输出。
    if shared.cancelled.load(Ordering::Acquire) {
        cancel.store(true, Ordering::SeqCst);
        return;
    }

    // 新的一轮开始：通知前端清空上一轮答案，并带上本轮问题与排队数。
    emit(
        app,
        EVT_ANSWER_DELTA,
        json!({
            "text": "",
            "reset": true,
            "regenerate": job.is_regenerate,
            "question": question,
            "pending": pending,
        }),
    );

    let provider = settings.provider.as_str().to_string();
    let key = match secret::get(&provider) {
        Ok(key) => key,
        Err(error) => {
            tracing::warn!(%error, "读取 API Key 失败");
            finish_answer(
                app,
                shared,
                &cancel,
                AnswerOutcome::Failed {
                    message: format!(
                        "读取 API Key 失败：{error}。请到设置页重新保存 Key 后点「重新生成」。"
                    ),
                    partial: String::new(),
                    ttft_ms: None,
                    retryable: false,
                },
                job.segment_id,
                // 带模型名，避免被前端当成「空 text + 空 model = 清空答案」。
                Some(settings.model.clone()),
            );
            return;
        }
    };

    // 旧流被重新生成 / 会话停止取代后，stream 内部要到下一个 delta 才会
    // 观察到取消；这里在 emit 前先看自己的取消标志，避免旧 delta 污染新答案。
    let cancel_for_delta = Arc::clone(&cancel);
    // 会话停止时 stop_session 只会取消「当时槽位里的那个」标志；如果本轮
    // 是在停止瞬间刚出队的，需要额外看会话级 cancelled，避免继续推 delta。
    let session_for_delta = Arc::clone(shared);
    let partial = Arc::new(Mutex::new(PartialAnswer::default()));
    let partial_for_delta = Arc::clone(&partial);
    let started = Instant::now();
    let result = llm::stream(&settings, &key, request, &cancel, |delta| {
        if cancel_for_delta.load(Ordering::Acquire)
            || session_for_delta.cancelled.load(Ordering::Acquire)
        {
            return;
        }
        {
            let mut state = lock(&partial_for_delta);
            if state.ttft_ms.is_none() && !delta.trim().is_empty() {
                state.ttft_ms = Some(started.elapsed().as_millis() as u64);
            }
            state.text.push_str(delta);
        }
        emit(app, EVT_ANSWER_DELTA, json!({ "text": delta }));
    })
    .await;

    let outcome = match result {
        Ok(stats) => AnswerOutcome::Completed(stats),
        Err(error) => {
            // 可重试性必须取类型化判定的结果，不能靠字符串里的状态码猜；
            // 例如「响应中没有可用文本」不含状态码，但并不可重试。
            let retryable = error.is_retryable();
            let state = lock(&partial);
            AnswerOutcome::Failed {
                message: error.to_string(),
                partial: state.text.clone(),
                ttft_ms: state.ttft_ms,
                retryable,
            }
        }
    };
    finish_answer(
        app,
        shared,
        &cancel,
        outcome,
        job.segment_id,
        Some(settings.model.clone()),
    );
}

/// 流式过程中的部分结果：失败时用来回填 answer-done，避免用户看到空白。
#[derive(Default)]
struct PartialAnswer {
    text: String,
    ttft_ms: Option<u64>,
}

/// 一轮回答的最终结果。
enum AnswerOutcome {
    Completed(llm::ChatStats),
    Failed {
        message: String,
        partial: String,
        ttft_ms: Option<u64>,
        retryable: bool,
    },
}

fn finish_answer(
    app: &AppHandle,
    shared: &Arc<SessionShared>,
    cancel: &Arc<AtomicBool>,
    outcome: AnswerOutcome,
    question_segment_id: Option<i64>,
    model_hint: Option<String>,
) {
    // 已被新一轮或停止会话取代：静默丢弃，避免踩掉正在流的答案。
    let still_current = {
        let slot = lock(&shared.answer_cancel);
        slot.as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, cancel))
    };
    // 取消不是错误：用户主动停止 / 重新生成，不能弹任何失败提示。
    if !still_current || shared.cancelled.load(Ordering::Acquire) || cancel.load(Ordering::Acquire)
    {
        return;
    }

    match outcome {
        AnswerOutcome::Completed(stats) => {
            let text = stats.text.trim().to_string();
            if !text.is_empty() {
                // 先写入上下文历史：下一题组装提示词时才能看到本题答案。
                shared.push_generated_answer(&text);
                let state = app.state::<AppState>();
                let store = lock(&state.store);
                if let Err(error) = store.add_answer(
                    shared.session_id,
                    question_segment_id,
                    &text,
                    if stats.model.trim().is_empty() {
                        model_hint.as_deref().unwrap_or("")
                    } else {
                        stats.model.as_str()
                    },
                    AnswerMetrics {
                        ttft_ms: Some(stats.ttft_ms as i64),
                        total_ms: Some(stats.total_ms as i64),
                        token_in: None,
                        token_out: None,
                    },
                ) {
                    tracing::warn!(%error, "写入答案失败");
                }
            }
            emit(
                app,
                EVT_ANSWER_DONE,
                json!({
                    "text": text,
                    "model": stats.model,
                    "ttftMs": stats.ttft_ms,
                }),
            );
            emit_status(app, "listening", "面试进行中");
        }
        AnswerOutcome::Failed {
            message,
            partial,
            ttft_ms,
            retryable,
        } => {
            // 会话状态保持 listening：音频正常时一次 LLM 失败不等于面试结束。
            tracing::warn!(%message, "生成回答失败");
            let partial = partial.trim().to_string();
            let action = if retryable {
                "点浮层「重新生成」或按 Ctrl+Shift+A 重试"
            } else {
                "请到设置页检查 API Key、模型名与接口地址后再试"
            };
            let detail = if partial.is_empty() {
                format!("网络或接口异常，还没有生成任何内容（{message}）；{action}。")
            } else {
                format!("生成中断（已保留已生成内容，{message}）；{action}。")
            };

            // 用 notice 而不是 status：不能覆盖会话的 listening 状态。
            emit_notice(app, "error", format!("生成回答失败：{detail}"));
            emit(
                app,
                EVT_ANSWER_DONE,
                json!({
                    "text": partial,
                    "model": model_hint.unwrap_or_default(),
                    "ttftMs": ttft_ms,
                    "error": detail,
                    "retryable": retryable,
                }),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question(text: &str) -> QueuedQuestion {
        QueuedQuestion {
            text: text.to_string(),
            segment_id: None,
            is_regenerate: false,
        }
    }

    #[test]
    fn queue_preserves_fifo_order() {
        let mut queue = QuestionQueue::default();
        assert_eq!(queue.enqueue(question("第一问")), EnqueueOutcome::Queued);
        assert_eq!(queue.enqueue(question("第二问")), EnqueueOutcome::Queued);
        assert!(queue.begin_worker());
        assert_eq!(queue.take_next().unwrap().text, "第一问");
        assert_eq!(queue.pending(), 1);
        assert_eq!(queue.take_next().unwrap().text, "第二问");
        assert!(queue.take_next().is_none());
    }

    #[test]
    fn dedupes_only_punctuation_and_politeness_variants() {
        let mut queue = QuestionQueue::default();
        assert_eq!(
            queue.enqueue(question("请介绍一下你自己？")),
            EnqueueOutcome::Queued
        );
        // 仅标点/空白不同，归一化后相同。
        assert_eq!(
            queue.enqueue(question("请 介绍 一下你自己")),
            EnqueueOutcome::Duplicate
        );
        // 去掉礼貌前缀后相同。
        assert_eq!(
            queue.enqueue(question("介绍一下你自己")),
            EnqueueOutcome::Duplicate
        );
        assert!(queue.begin_worker());
        let current = queue.take_next().unwrap();
        assert_eq!(current.text, "请介绍一下你自己？");
        // 与「当前正在作答」重复。
        assert_eq!(
            queue.enqueue(question("请介绍一下你自己")),
            EnqueueOutcome::Duplicate
        );
        queue.finish_current();
        // 「是什么」与「是什么呀」仅语气尾缀不同。
        assert_eq!(
            queue.enqueue(question("你最大的优点是什么")),
            EnqueueOutcome::Queued
        );
        assert_eq!(
            queue.enqueue(question("你最大的优点是什么呀")),
            EnqueueOutcome::Duplicate
        );
    }

    /// 真实追问不能被当成重复吞掉——这是连续多问场景的头号要求。
    #[test]
    fn genuine_follow_up_question_is_not_deduped() {
        let mut queue = QuestionQueue::default();
        assert_eq!(
            queue.enqueue(question("介绍一下你的项目")),
            EnqueueOutcome::Queued
        );
        assert_eq!(
            queue.enqueue(question("介绍一下你的项目难点")),
            EnqueueOutcome::Queued
        );
        assert_eq!(
            queue.enqueue(question("你最大的优点是什么")),
            EnqueueOutcome::Queued
        );
        assert_eq!(
            queue.enqueue(question("你最大的缺点是什么")),
            EnqueueOutcome::Queued
        );
        assert_eq!(queue.pending(), 4);
    }

    #[test]
    fn queue_limit_drops_new_question() {
        let mut queue = QuestionQueue::default();
        for index in 0..ANSWER_QUEUE_LIMIT {
            assert_eq!(
                queue.enqueue(question(&format!("问题{index}"))),
                EnqueueOutcome::Queued
            );
        }
        assert_eq!(queue.pending(), ANSWER_QUEUE_LIMIT);
        assert_eq!(queue.enqueue(question("第六个问题")), EnqueueOutcome::Full);
        assert_eq!(queue.pending(), ANSWER_QUEUE_LIMIT);
    }

    #[test]
    fn clear_removes_items_but_not_worker_flag() {
        let mut queue = QuestionQueue::default();
        assert_eq!(queue.enqueue(question("问题")), EnqueueOutcome::Queued);
        assert!(queue.begin_worker());
        queue.clear();
        assert_eq!(queue.pending(), 0);
        assert!(queue.take_next().is_none());
        // clear 不应把 worker 标志置回 false，否则会并发拉起第二个 worker。
        assert!(!queue.begin_worker());
        assert!(!queue.end_worker());
    }

    #[test]
    fn worker_flag_prevents_duplicate_workers() {
        let mut queue = QuestionQueue::default();
        assert!(!queue.begin_worker());
        assert_eq!(queue.enqueue(question("问题")), EnqueueOutcome::Queued);
        assert!(queue.begin_worker());
        assert_eq!(
            queue.enqueue(question("第二个问题")),
            EnqueueOutcome::Queued
        );
        assert!(!queue.begin_worker());
    }

    #[test]
    fn normalization_and_similarity_handle_chinese_punctuation() {
        assert_eq!(normalize_for_dedup("你好，世界。 Hello!"), "你好世界hello");
        assert!(similar_text(
            &normalize_for_dedup("你好，世界。"),
            &normalize_for_dedup("你好世界")
        ));
        assert!(!similar_text("我的项目经验", "我的项目经验很"));
        assert!(!similar_text("我的项目", "完全不同的问题"));
        assert!(!similar_text("", "空"));
    }
}
