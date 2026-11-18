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

use hint_audio::{AudioChunk, AudioConfig, AudioEngine, Channel, Levels};
use hint_core::llm::{self, ChatRequest, Turn, TurnRole};
use hint_core::pipeline;
use hint_core::secret;
use hint_core::settings::{Settings, TriggerMode};
use hint_core::session::{LifecycleAction, LifecycleDir};
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

const TARGET_RATE: u32 = 16_000;
/// 判定「有语音」的 RMS 阈值。
const SPEECH_RMS: f32 = 0.012;
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

    fn rms(samples: &[f32]) -> f32 {
        let mut sum = 0.0f64;
        let mut count = 0u64;
        for &sample in samples {
            if sample.is_finite() {
                sum += f64::from(sample) * f64::from(sample);
                count += 1;
            }
        }
        if count == 0 {
            0.0
        } else {
            (sum / count as f64).sqrt() as f32
        }
    }

    fn push(&mut self, chunk: &[f32], out: &mut Vec<RawSegment>) {
        let chunk_ms = samples_to_ms(chunk.len());
        let loud = Self::rms(chunk) >= SPEECH_RMS;

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
            return;
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
        ctx.recent_interviewer
            .iter()
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
    let action = {
        let state = app.state::<AppState>();
        lock(&state.lifecycle).request(dir)
    };
    let result = match action {
        LifecycleAction::Run(d) => run_lifecycle(app, d),
        _ => Ok(0),
    };
    (action, result)
}

/// 执行一个生命周期动作：begin → 执行 → finish → drain 排队请求。
fn run_lifecycle(app: &AppHandle, dir: LifecycleDir) -> Result<i64, String> {
    {
        let state = app.state::<AppState>();
        lock(&state.lifecycle).begin(dir);
    }
    let result = match dir {
        LifecycleDir::Start => start_session(app),
        LifecycleDir::Stop => stop_session(app).map(|_| 0),
    };
    {
        let state = app.state::<AppState>();
        lock(&state.lifecycle).finish(dir, result.is_ok());
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
            queue.next_pending()
        };
        let Some(dir) = next else { return };
        // 排队请求的执行结果无需返回给调用方，只需完成。
        let _ = run_lifecycle(app, dir);
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
        move |chunk: AudioChunk| {
            if shared.cancelled.load(Ordering::Acquire) {
                return;
            }
            let mut finished = Vec::new();
            {
                let mut guard = lock(&segmenters);
                let segmenter = match chunk.channel {
                    Channel::Interviewer => &mut guard.interviewer,
                    Channel::Me => &mut guard.me,
                };
                segmenter.push(&chunk.samples, &mut finished);
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

    let handle = match AudioEngine::start(config, on_chunk, on_levels, on_error) {
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
fn end_session_after_audio_failure(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let task_app = app.clone();
        match tauri::async_runtime::spawn_blocking(move || stop_session(&task_app)).await {
            Ok(Ok(())) => {
                emit_status(&app, "error", "音频采集中断，会话已结束");
            }
            Ok(Err(error)) => tracing::warn!(%error, "音频中断后结束会话失败"),
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
    client: WorkerClient,
    done: Arc<(Mutex<bool>, std::sync::Condvar)>,
) {
    loop {
        if shared.cancelled.load(Ordering::Acquire) {
            break;
        }
        match jobs.recv_timeout(Duration::from_secs(1)) {
            Ok(job) => {
                let result = client.transcribe(&job.wav, "auto");
                match result {
                    Ok(transcript) => {
                        let text = transcript.text.trim().to_string();
                        if !text.is_empty() {
                            handle_transcript(
                                &app,
                                &shared,
                                job.channel,
                                &text,
                                job.start_ms,
                                job.end_ms,
                            );
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "本地语音识别失败");
                        if !shared.cancelled.load(Ordering::Acquire) {
                            emit_status(
                                &app,
                                "listening",
                                format!("语音识别失败，稍后自动重试：{}", user_message_for(&error)),
                            );
                        }
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if client.idle_for().is_some_and(|idle| idle > WORKER_IDLE) {
                    tracing::info!("空闲超过 5 分钟，回收语音 worker");
                    client.stop();
                }
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
    let partial = Arc::new(Mutex::new(PartialAnswer::default()));
    let partial_for_delta = Arc::clone(&partial);
    let started = Instant::now();
    let result = llm::stream(&settings, &key, request, &cancel, |delta| {
        if cancel_for_delta.load(Ordering::Acquire) {
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
