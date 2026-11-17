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
    ctx: Mutex<AnswerCtx>,
}

impl SessionShared {
    fn push_turn(&self, role: TurnRole, text: &str) {
        let mut ctx = lock(&self.ctx);
        ctx.history.push(Turn {
            role,
            text: text.to_string(),
        });
        let overflow = ctx.history.len().saturating_sub(HISTORY_LIMIT);
        if overflow > 0 {
            ctx.history.drain(..overflow);
        }
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

/// 重新生成最近一个问题的回答。
pub(crate) fn regenerate(app: &AppHandle) -> Result<(), String> {
    let shared = current_shared(app).ok_or_else(|| "请先开始面试会话。".to_string())?;
    let question = shared
        .last_question()
        .ok_or_else(|| "暂无可回答的问题，请等待面试官提问。".to_string())?;
    let mut history = shared.history_snapshot();
    // 去掉与本次问题重复的尾轮，避免提示词里出现两遍。
    if let Some(last) = history.last()
        && last.role == TurnRole::Interviewer
        && last.text == question
    {
        history.pop();
    }
    spawn_answer(app.clone(), shared, question, history, None, false);
    Ok(())
}

// 托盘 / 热键用的异步入口：内部全是阻塞操作，必须离开事件循环线程。
pub(crate) fn spawn_start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let task_app = app.clone();
        if let Err(error) = tauri::async_runtime::spawn_blocking(move || start_session(&task_app))
            .await
            .map_err(|error| format!("启动任务失败：{error}"))
            .and_then(|result| result)
        {
            tracing::warn!(%error, "托盘/热键启动会话失败");
            emit_status(&app, "error", error);
        }
    });
}

pub(crate) fn spawn_stop(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let task_app = app.clone();
        if let Err(error) = tauri::async_runtime::spawn_blocking(move || stop_session(&task_app))
            .await
            .map_err(|error| format!("停止任务失败：{error}"))
            .and_then(|result| result)
        {
            tracing::warn!(%error, "托盘/热键结束会话失败");
            emit_status(&app, "error", error);
        }
    });
}

pub(crate) fn spawn_regenerate(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = regenerate(&app) {
            emit_status(&app, "error", error);
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
            let history = shared.history_snapshot();
            shared.push_turn(TurnRole::Interviewer, text);
            shared.record_interviewer(text, is_question);

            if is_question && pipeline::should_generate(shared.trigger, true) {
                spawn_answer(
                    app.clone(),
                    Arc::clone(shared),
                    text.to_string(),
                    history,
                    segment_id,
                    false,
                );
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

fn spawn_answer(
    app: AppHandle,
    shared: Arc<SessionShared>,
    question: String,
    history: Vec<Turn>,
    question_segment_id: Option<i64>,
    is_regenerate: bool,
) {
    if shared.cancelled.load(Ordering::Acquire) {
        return;
    }
    let settings = shared.settings.clone();
    let system = pipeline::build_system_prompt(&settings);
    let user = pipeline::build_user_prompt(&question, &history);
    let request = ChatRequest {
        system,
        user,
        ..ChatRequest::default()
    };

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = lock(&shared.answer_cancel);
        if let Some(previous) = slot.replace(Arc::clone(&cancel)) {
            previous.store(true, Ordering::SeqCst);
        }
    }

    // 新的一轮开始：通知前端清空上一轮答案。
    emit(
        &app,
        EVT_ANSWER_DELTA,
        json!({ "text": "", "reset": true, "regenerate": is_regenerate }),
    );

    let delta_app = app.clone();
    let done_app = app.clone();
    let shared_for_task = Arc::clone(&shared);
    let cancel_for_task = Arc::clone(&cancel);
    let model_id = settings.model.clone();

    tauri::async_runtime::spawn(async move {
        let provider = settings.provider.as_str().to_string();
        let key = match secret::get(&provider) {
            Ok(key) => key,
            Err(error) => {
                tracing::warn!(%error, "读取 API Key 失败");
                finish_answer(
                    &done_app,
                    &shared_for_task,
                    &cancel_for_task,
                    Err(format!("读取 API Key 失败：{error}")),
                    None,
                    None,
                );
                return;
            }
        };

        // 旧流被新问题取代（或会话停止）后，stream 内部要到下一个 delta 才会
        // 观察到取消；这里在 emit 前先看自己的取消标志，避免旧 delta 污染新答案。
        let cancel_for_delta = Arc::clone(&cancel_for_task);
        let result = llm::stream(&settings, &key, request, &cancel_for_task, |delta| {
            if cancel_for_delta.load(Ordering::Acquire) {
                return;
            }
            emit(&delta_app, EVT_ANSWER_DELTA, json!({ "text": delta }));
        })
        .await;

        finish_answer(
            &done_app,
            &shared_for_task,
            &cancel_for_task,
            result.map_err(|error| error.to_string()),
            question_segment_id,
            Some(model_id),
        );
    });
}

fn finish_answer(
    app: &AppHandle,
    shared: &Arc<SessionShared>,
    cancel: &Arc<AtomicBool>,
    result: Result<llm::ChatStats, String>,
    question_segment_id: Option<i64>,
    model_hint: Option<String>,
) {
    // 已被新一轮或停止会话取代：静默丢弃，避免踩掉正在流的答案。
    let still_current = {
        let slot = lock(&shared.answer_cancel);
        slot.as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, cancel))
    };
    if !still_current || shared.cancelled.load(Ordering::Acquire) {
        return;
    }

    match result {
        Ok(stats) => {
            let text = stats.text.trim().to_string();
            if !text.is_empty() {
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
                json!({ "text": text, "model": stats.model, "ttftMs": stats.ttft_ms }),
            );
            emit_status(app, "listening", "面试进行中");
        }
        Err(message) => {
            tracing::warn!(%message, "生成回答失败");
            emit_status(app, "error", format!("生成回答失败：{message}"));
            emit(
                app,
                EVT_ANSWER_DONE,
                json!({ "text": "", "model": "", "ttftMs": null }),
            );
        }
    }
}
