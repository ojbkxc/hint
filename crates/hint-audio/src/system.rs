//! 系统声采集线程（Windows WASAPI loopback）。
//!
//! 实现方式（参照 pluely / natively 的成熟做法，按 wasapi 0.27 API 重写）：
//! 1. 在采集线程内初始化 COM（MTA），线程退出时反初始化；
//! 2. 在 render 端点上打开 `Direction::Capture` 的共享、事件驱动流，
//!    即 WASAPI loopback；`autoconvert` 允许把设备混音格式转换为
//!    f32 / 指定的声道数；
//! 3. 用 `IAudioCaptureClient::read_from_device_to_deque` 逐包读取；
//! 4. 事件超时（系统静音、没有程序在播放）时按实际流逝时间补静音帧，
//!    保持下游 VAD / 切块节奏连续；
//! 5. 端点回退顺序：指定设备 → 默认 render（Console 角色）→
//!    通信设备（Communications 角色）；未显式指定设备时，运行中跟随
//!    系统默认输出设备变更（每 3 秒检查一次）；
//! 6. 读取连续失败 5 次或事件长时间不触发时，上报错误并重建会话；
//!    连续 5 次会话失败判定为致命并停止整个引擎。

use crate::Channel;
use crate::pipeline::ChannelPipeline;
use crate::runtime::Runtime;
use std::sync::Arc;
use std::time::{Duration, Instant};
use wasapi::{
    AudioCaptureClient, AudioClient, AudioClientProperties, Device, DeviceEnumerator, DeviceState,
    Direction, Handle, Role, SampleType, StreamMode, StreamOption, WasapiError, WaveFormat,
};

/// 连续读取错误多少次后判定会话失效（触发重连）。
const MAX_READ_ERRORS: usize = 5;
/// 连续会话失败多少次后判定通道死亡。
const MAX_CONSECUTIVE_FAILURES: u32 = 5;
/// 失败至少要持续这么久才判致命：切换默认设备/蓝牙断开重连的短暂失败
/// 不应终止整场面试。真正拔掉的设备仍会在宽限后停止。
const FATAL_GRACE: Duration = Duration::from_secs(15);
/// 流稳定运行多久后清零失败计数。
const HEALTHY_AFTER: Duration = Duration::from_secs(3);
/// 事件等待超时；超时视为静音期并按流逝时间补静音帧。
const EVENT_WAIT_MS: u32 = 200;
/// 端点存活检查间隔。
const DEVICE_STATE_CHECK_INTERVAL: Duration = Duration::from_secs(3);

/// 系统声采集线程入口。
pub(crate) fn run(
    rt: Arc<Runtime>,
    init: std::sync::mpsc::Sender<std::result::Result<(), String>>,
) {
    let mut init = Some(init);

    let _com = match ComGuard::initialize() {
        Ok(guard) => guard,
        Err(message) => {
            if let Some(tx) = init.take() {
                let _ = tx.send(Err(message));
            } else {
                rt.error(message);
            }
            rt.finish_channel(Channel::Interviewer);
            return;
        }
    };

    let mut failures: u32 = 0;
    let mut first_failure_at: Option<Instant> = None;
    let mut degraded_hinted = false;

    loop {
        if rt.should_stop() {
            break;
        }
        match open_session(&rt) {
            Ok(mut session) => {
                if let Some(tx) = init.take() {
                    let _ = tx.send(Ok(()));
                }
                tracing::info!(
                    "系统声 loopback 已启动：{} Hz，{} 声道，格式={}，RAW={}",
                    session.rate,
                    session.channels,
                    session.format_label,
                    if session.raw_accepted {
                        "已绕开音效"
                    } else {
                        "未绕开（音效会改写波形）"
                    }
                );
                let started = Instant::now();
                match session.run(&rt) {
                    Ok(SessionEnd::Stopped) => break,
                    Ok(SessionEnd::Migrated) => {
                        // 用户换了默认输出设备：走「警告」而不是「错误」，
                        // 也不计入连续失败——跟着端点迁移是正常路径，
                        // 插拔几次耳机就把引擎判死是错的。
                        rt.warning("系统默认输出设备已变更，正在切换到新的输出设备继续采集");
                        failures = 0;
                        first_failure_at = None;
                        degraded_hinted = false;
                    }
                    Err(message) => {
                        rt.error(format!("系统声采集中断：{message}"));
                        // 运行足够久说明设备本身没问题，重新计数从 1 开始。
                        failures = if started.elapsed() >= HEALTHY_AFTER {
                            first_failure_at = Some(Instant::now());
                            1
                        } else {
                            first_failure_at.get_or_insert_with(Instant::now);
                            failures.saturating_add(1)
                        };
                    }
                }
            }
            Err(message) => {
                if let Some(tx) = init.take() {
                    // 首次启动失败：交给 AudioEngine::start 统一上报并返回错误。
                    let _ = tx.send(Err(message));
                    break;
                }
                rt.error(format!("系统声设备重新打开失败：{message}"));
                failures = failures.saturating_add(1);
                first_failure_at.get_or_insert_with(Instant::now);
            }
        }

        if rt.should_stop() {
            break;
        }
        if failures >= MAX_CONSECUTIVE_FAILURES && failure_grace_exceeded(first_failure_at) {
            rt.error(format!(
                "系统声通道连续 {MAX_CONSECUTIVE_FAILURES} 次失败，采集引擎停止"
            ));
            rt.fatal();
            break;
        }
        if failures == DEGRADED_HINT_AFTER && !degraded_hinted {
            degraded_hinted = true;
            rt.warning(
                "系统声通道反复失败，可能听不到面试官；请检查输出设备，\
                 必要时结束会话后在设置页重新选择设备",
            );
        }
        // 断线期间没有音频：清电平、重置迟滞检测器并让联合状态回到静默，
        // 避免重连后同向边被吸收、状态卡在「面试官在说」。
        rt.finish_channel(Channel::Interviewer);
        rt.sleep_backoff(failures);
    }

    rt.finish_channel(Channel::Interviewer);
}

/// 连续失败是否已持续超过 `FATAL_GRACE`。
fn failure_grace_exceeded(first_failure_at: Option<Instant>) -> bool {
    first_failure_at.is_some_and(|at| at.elapsed() >= FATAL_GRACE)
}

/// 失败次数达到该值时先给一次「可能听不到面试官」的非致命提示。
const DEGRADED_HINT_AFTER: u32 = 3;

/// COM(MTA) 初始化守卫：只在初始化成功的线程上反初始化。
struct ComGuard {
    active: bool,
}

impl ComGuard {
    fn initialize() -> std::result::Result<Self, String> {
        let result = wasapi::initialize_mta();
        if result.is_err() {
            return Err(format!("初始化 COM（MTA）失败：0x{:08X}", result.0 as u32));
        }
        Ok(Self { active: true })
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.active {
            // 与 initialize_mta 成对调用，避免线程 COM 引用计数泄漏。
            wasapi::deinitialize();
        }
    }
}

/// 会话使用的样本编码。
#[derive(Clone, Copy, Debug)]
enum WasapiSample {
    F32,
    F64,
    I16,
    I32,
}

impl WasapiSample {
    fn bytes(self) -> usize {
        match self {
            WasapiSample::F32 => 4,
            WasapiSample::F64 => 8,
            WasapiSample::I16 => 2,
            WasapiSample::I32 => 4,
        }
    }
}

/// 一次已建立的 loopback 会话。
struct LoopbackSession {
    client: AudioClient,
    capture: AudioCaptureClient,
    event: Handle,
    device: Device,
    rate: u32,
    channels: usize,
    sample: WasapiSample,
    /// 实际协商到的格式名（`设备混音格式` / `混音声道数 f32` / `单声道 f32`）。
    ///
    /// 落日志用：前两种会自动转换（可能经音效处理改写波形），最后一种才是设备
    /// 原始样本。排查「识别成乱码」时需要一眼看出走的是哪条路径。
    format_label: &'static str,
    /// 是否成功请求到 RAW 流（绕开音效处理）。仅在 `设备混音格式` 这一档尝试。
    ///
    /// 排查用：若这里是 `false` 而识别仍乱，说明音效绕不过去，方向要转成
    /// 「引导用户关音效」或「换采集方式」，而不是继续调格式。
    raw_accepted: bool,
    /// 未显式指定输出设备时，跟随系统默认设备变更。
    follow_default: bool,
    /// 会话打开时的系统默认 render 端点 ID；用于检测「用户换了默认设备」。
    default_at_open: Option<String>,
}

/// 会话正常结束的原因。
///
/// 用户切换系统默认输出设备是**正常路径**：必须和「会话失效」区分开，
/// 否则一次插拔耳机就会被记成一次通道失败（并弹一条「采集中断」）。
enum SessionEnd {
    /// 收到停止请求。
    Stopped,
    /// 系统默认输出设备变了，需要按新端点重建会话。
    Migrated,
}

impl LoopbackSession {
    /// 读取循环；`Ok` 表示正常结束，`Err` 表示会话失效需重建。
    fn run(&mut self, rt: &Runtime) -> std::result::Result<SessionEnd, String> {
        let mut pipeline = ChannelPipeline::new(
            Channel::Interviewer,
            self.rate,
            rt.config.target_sample_rate,
            rt.config.chunk_samples,
        )
        .map_err(|e| format!("系统声重采样初始化失败：{e}"))?;

        let mut bytes: std::collections::VecDeque<u8> = std::collections::VecDeque::new();
        let mut mono: Vec<f32> = Vec::with_capacity(16 * 1024);
        let mut last_emit = Instant::now();
        let mut consecutive_errors: usize = 0;
        let mut last_state_check = Instant::now();

        loop {
            if rt.should_stop() {
                break;
            }

            // 端点存活检查：设备拔出/禁用时尽快返回错误并触发重连。
            if last_state_check.elapsed() >= DEVICE_STATE_CHECK_INTERVAL {
                last_state_check = Instant::now();
                match self.device.get_state() {
                    Ok(DeviceState::Active) => {}
                    Ok(state) => {
                        return Err(format!("系统输出设备状态异常：{state:?}"));
                    }
                    Err(error) => {
                        return Err(format!("查询系统输出设备状态失败：{error}"));
                    }
                }
                // 默认输出设备被用户切换时，主动迁移到新端点。
                if self.follow_default && default_render_id_changed(&self.device) {
                    return Ok(SessionEnd::Migrated);
                }
                // 切换默认设备后，旧 loopback 端点常常仍然「存活」但只回静音；
                // 若显式指定了设备或回退到了通信设备（不跟随默认），这里检测到
                // 默认端点变化却无法自动迁移，必须明确告知用户怎么恢复。
                if !self.follow_default
                    && let Some(opened) = self.default_at_open.as_deref()
                    && current_default_render_id().is_some_and(|now| now != opened)
                {
                    self.default_at_open = None; // 只提醒一次
                    rt.warning(
                        "系统默认输出设备已变更，当前会话仍在采集旧设备，可能听不到面试官；\
                         请结束会话，在设置中重新选择输出设备并重新开始",
                    );
                }
            }

            let mut received = false;

            match self.event.wait_for_event(EVENT_WAIT_MS) {
                Ok(()) => {
                    received = self.drain_packets(
                        rt,
                        &mut pipeline,
                        &mut bytes,
                        &mut mono,
                        &mut last_emit,
                        &mut consecutive_errors,
                    )?;
                }
                Err(WasapiError::EventTimeout) => {
                    // 超时既可能是静音期，也可能是端点已失效。
                    // 主动查询一次包大小，让 DEVICE_INVALIDATED 等错误暴露出来。
                    match self.capture.get_next_packet_size() {
                        Ok(Some(frames)) if frames > 0 => {
                            received = self.drain_packets(
                                rt,
                                &mut pipeline,
                                &mut bytes,
                                &mut mono,
                                &mut last_emit,
                                &mut consecutive_errors,
                            )?;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            consecutive_errors = consecutive_errors.saturating_add(1);
                            if consecutive_errors >= MAX_READ_ERRORS {
                                return Err(format!("系统声事件超时且读取失败：{error}"));
                            }
                        }
                    }
                }
                Err(error) => {
                    return Err(format!("等待系统声事件失败：{error}"));
                }
            }

            if !received {
                emit_silence(rt, &mut pipeline, self.rate, &mut last_emit)?;
            }
        }

        Ok(SessionEnd::Stopped)
    }

    /// 读空当前所有音频包；返回是否读到非空数据。
    #[allow(clippy::too_many_arguments)]
    fn drain_packets(
        &mut self,
        rt: &Runtime,
        pipeline: &mut ChannelPipeline,
        bytes: &mut std::collections::VecDeque<u8>,
        mono: &mut Vec<f32>,
        last_emit: &mut Instant,
        consecutive_errors: &mut usize,
    ) -> std::result::Result<bool, String> {
        let mut received = false;
        loop {
            let packet = match self.capture.get_next_packet_size() {
                Ok(Some(frames)) => frames as usize,
                Ok(None) => 0,
                Err(error) => {
                    *consecutive_errors = consecutive_errors.saturating_add(1);
                    if *consecutive_errors >= MAX_READ_ERRORS {
                        return Err(format!("读取系统声包大小连续失败：{error}"));
                    }
                    break;
                }
            };
            if packet == 0 {
                break;
            }

            bytes.clear();
            match self.capture.read_from_device_to_deque(bytes) {
                Ok(_info) => {
                    *consecutive_errors = 0;
                    if bytes.is_empty() {
                        break;
                    }
                    let contiguous = bytes.make_contiguous();
                    bytes_to_mono(contiguous, self.channels, self.sample, mono)
                        .map_err(|e| format!("系统声格式转换失败：{e}"))?;
                    if !mono.is_empty() {
                        let chunks = pipeline
                            .push_mono(mono)
                            .map_err(|e| format!("系统声重采样失败：{e}"))?;
                        for chunk in chunks {
                            rt.publish_chunk(chunk);
                        }
                        received = true;
                        *last_emit = Instant::now();
                    }
                }
                Err(error) => {
                    *consecutive_errors = consecutive_errors.saturating_add(1);
                    if *consecutive_errors >= MAX_READ_ERRORS {
                        return Err(format!("读取系统声数据连续失败：{error}"));
                    }
                    break;
                }
            }
        }
        Ok(received)
    }
}

impl Drop for LoopbackSession {
    fn drop(&mut self) {
        // 会话重连/线程退出时尽量显式停流；失败不影响后续重建。
        if let Err(error) = self.client.stop_stream() {
            tracing::debug!("停止 loopback 流失败（可忽略）：{error}");
        }
    }
}

/// 按实际流逝时间补静音帧，保持下游时间轴连续（约每 20 ms 检查一次）。
fn emit_silence(
    rt: &Runtime,
    pipeline: &mut ChannelPipeline,
    rate: u32,
    last_emit: &mut Instant,
) -> std::result::Result<(), String> {
    let elapsed = last_emit.elapsed();
    if elapsed < Duration::from_millis(20) {
        return Ok(());
    }
    let frames = (f64::from(rate) * elapsed.as_secs_f64()).round() as usize;
    // 单次补帧上限 1 秒，避免长时间挂起后一次性灌入过多静音。
    let frames = frames.clamp(1, rate.max(1) as usize);
    let silence = vec![0.0f32; frames];
    for chunk in pipeline
        .push_mono(&silence)
        .map_err(|e| format!("系统声静音补帧失败：{e}"))?
    {
        rt.publish_chunk(chunk);
    }
    *last_emit = Instant::now();
    Ok(())
}

/// 打开一次 loopback 会话（含设备回退与格式回退）。
fn open_session(rt: &Runtime) -> std::result::Result<LoopbackSession, String> {
    let enumerator = DeviceEnumerator::new().map_err(|e| format!("创建音频设备枚举器失败：{e}"))?;
    let candidates = candidate_devices(&enumerator, rt.config.output_device.as_deref(), rt);
    if candidates.is_empty() {
        return Err("没有可用的系统输出设备".to_string());
    }
    let follow_default = rt
        .config
        .output_device
        .as_deref()
        .filter(|value| !value.is_empty())
        .is_none();

    let mut last_error = String::from("未知错误");
    for (label, device) in candidates {
        match init_loopback(device) {
            Ok(mut session) => {
                // 只有实际使用「默认输出设备」时才跟随默认设备变更；
                // 显式指定的设备或通信设备回退不跟随。
                session.follow_default = follow_default && label == "默认输出设备";
                // 记录打开时的默认端点，便于在无法自动跟随的情形下提示用户。
                session.default_at_open = if session.follow_default {
                    None
                } else {
                    current_default_render_id()
                };
                return Ok(session);
            }
            Err(error) => {
                tracing::warn!("系统声设备「{label}」初始化失败：{error}");
                last_error = format!("{label}：{error}");
            }
        }
    }
    Err(format!("所有候选输出设备均初始化失败：{last_error}"))
}

/// 当前默认 render 端点的稳定 ID；拿不到时返回 `None`。
fn current_default_render_id() -> Option<String> {
    let enumerator = DeviceEnumerator::new().ok()?;
    let device = enumerator.get_default_device(Direction::Render).ok()?;
    device.get_id().ok()
}

/// 当前默认 render 端点是否已不同于本会话的设备。
fn default_render_id_changed(session_device: &Device) -> bool {
    let Ok(enumerator) = DeviceEnumerator::new() else {
        return false;
    };
    let Ok(default_device) = enumerator.get_default_device(Direction::Render) else {
        return false;
    };
    let Ok(default_id) = default_device.get_id() else {
        return false;
    };
    let Ok(session_id) = session_device.get_id() else {
        return false;
    };
    default_id != session_id
}

/// 候选设备顺序：指定设备 → 默认 render（Console）→ 通信设备。
fn candidate_devices(
    enumerator: &DeviceEnumerator,
    wanted: Option<&str>,
    rt: &Runtime,
) -> Vec<(String, Device)> {
    let mut out: Vec<(String, Device)> = Vec::new();
    let mut seen_ids: Vec<String> = Vec::new();

    if let Some(wanted) = wanted.filter(|value| !value.is_empty()) {
        let raw = wanted.strip_prefix("wasapi:").unwrap_or(wanted);
        // 只在 render 集合里找：`enumerator.get_device` 也会返回输入端点，
        // 把麦克风 ID 当输出设备会让 loopback 初始化以晦涩错误失败。
        let found = find_render_by_id(enumerator, raw)
            .or_else(|_| find_render_by_id(enumerator, wanted))
            .or_else(|_| find_by_name(enumerator, wanted));
        match found {
            Ok(device) => push_candidate(&mut out, &mut seen_ids, "指定设备", device),
            Err(_) => rt.warning(format!(
                "未找到指定的系统输出设备「{wanted}」，已回退到默认设备"
            )),
        }
    }

    if let Ok(device) = enumerator.get_default_device(Direction::Render) {
        push_candidate(&mut out, &mut seen_ids, "默认输出设备", device);
    }
    if let Ok(device) =
        enumerator.get_default_device_for_role(Direction::Render, Role::Communications)
    {
        push_candidate(&mut out, &mut seen_ids, "通信输出设备", device);
    }

    out
}

fn push_candidate(
    out: &mut Vec<(String, Device)>,
    seen_ids: &mut Vec<String>,
    label: &str,
    device: Device,
) {
    if let Ok(id) = device.get_id() {
        if seen_ids.iter().any(|seen| seen == &id) {
            return;
        }
        seen_ids.push(id);
    }
    out.push((label.to_string(), device));
}

/// 在 render 设备集合中按端点 ID 查找（避免把输入端点误当输出）。
fn find_render_by_id(
    enumerator: &DeviceEnumerator,
    id: &str,
) -> std::result::Result<Device, WasapiError> {
    let collection = enumerator.get_device_collection(Direction::Render)?;
    let count = collection.get_nbr_devices()?;
    for index in 0..count {
        let device = collection.get_device_at_index(index)?;
        if device.get_id().is_ok_and(|found| found == id) {
            return Ok(device);
        }
    }
    Err(WasapiError::DeviceNotFound(id.to_string()))
}

fn find_by_name(
    enumerator: &DeviceEnumerator,
    name: &str,
) -> std::result::Result<Device, WasapiError> {
    let collection = enumerator.get_device_collection(Direction::Render)?;
    let count = collection.get_nbr_devices()?;
    for index in 0..count {
        let device = collection.get_device_at_index(index)?;
        if device
            .get_friendlyname()
            .map(|friendly| friendly == name)
            .unwrap_or(false)
        {
            return Ok(device);
        }
    }
    Err(WasapiError::DeviceNotFound(name.to_string()))
}

/// 依次尝试三种格式：**设备原始混音格式优先**，其次混音声道数 f32，最后单声道 f32。
///
/// 顺序为什么是这样（真机抓音频后改的）：
/// 前两种带 `autoconvert: true`，等于请 Windows 音频引擎把设备混音格式**转换**
/// 成我们要的形态。这条转换路径会经过注册在端点上的音效处理（APO，Realtek /
/// Dolby / DTS 之类），波形会被实际改写——实测：采集回来的音频与原句的互相关
/// 只有 -0.08（几乎不相关），但频谱看着正常（语音频带占 70%），于是「人能听懂、
/// 模型认不出」，中文尤甚（声调依赖精确的基频轮廓）。
///
/// 用设备原始格式（`autoconvert: false`）就不请求这层转换，尽量拿到端点吐出的
/// 原始样本。它失败时才回退到会自动转换的两种——宁可音质被处理，也不能连不上。
fn init_loopback(device: Device) -> std::result::Result<LoopbackSession, String> {
    let attempts = [
        FormatAttempt::MixFormat,
        FormatAttempt::MixChannelFloat,
        FormatAttempt::MonoFloat,
    ];
    let mut last_error = String::from("未知错误");
    for attempt in attempts {
        match init_with_attempt(&device, attempt) {
            Ok(parts) => {
                return Ok(LoopbackSession {
                    client: parts.client,
                    capture: parts.capture,
                    event: parts.event,
                    device,
                    rate: parts.rate,
                    channels: parts.channels,
                    sample: parts.sample,
                    format_label: attempt.label(),
                    raw_accepted: parts.raw_accepted,
                    follow_default: false,
                    default_at_open: None,
                });
            }
            Err(error) => last_error = format!("{}：{error}", attempt.label()),
        }
    }
    Err(last_error)
}

/// 单次格式尝试的初始化产物（尚未绑定设备所有权）。
struct LoopbackParts {
    client: AudioClient,
    capture: AudioCaptureClient,
    event: Handle,
    rate: u32,
    channels: usize,
    sample: WasapiSample,
    /// RAW 流是否被接受（见 `LoopbackSession::raw_accepted`）。
    raw_accepted: bool,
}

#[derive(Clone, Copy, Debug)]
enum FormatAttempt {
    MonoFloat,
    MixChannelFloat,
    MixFormat,
}

impl FormatAttempt {
    fn label(self) -> &'static str {
        match self {
            FormatAttempt::MonoFloat => "单声道 f32",
            FormatAttempt::MixChannelFloat => "混音声道数 f32",
            FormatAttempt::MixFormat => "设备混音格式",
        }
    }
}

fn init_with_attempt(
    device: &Device,
    attempt: FormatAttempt,
) -> std::result::Result<LoopbackParts, String> {
    let mut client = device
        .get_iaudioclient()
        .map_err(|e| format!("获取 IAudioClient 失败：{e}"))?;
    let mix_format = client
        .get_mixformat()
        .map_err(|e| format!("获取设备混音格式失败：{e}"))?;
    let rate = mix_format.get_samplespersec();
    let mix_channels = mix_format.get_nchannels() as usize;
    let (_default_period, min_period) = client
        .get_device_period()
        .map_err(|e| format!("获取设备周期失败：{e}"))?;
    // 事件驱动共享模式只要求 buffer 不小于最小周期；但贴着最小值申请等于
    // 零余量：采集线程晚一两个调度周期（蓝牙/USB 抖动、系统忙）WASAPI 就会
    // 覆盖未读的包，表现为「偶发漏字」而不是报错。取 min_period 与 200ms 的
    // 较大者，异常大的上报周期仍封顶 1 秒。
    let buffer_duration = min_period.max(2_000_000).clamp(10_000, 10_000_000);

    let (format, channels, sample, autoconvert) = match attempt {
        FormatAttempt::MonoFloat => (
            WaveFormat::new(32, 32, SampleType::Float, rate as usize, 1, None),
            1,
            WasapiSample::F32,
            true,
        ),
        FormatAttempt::MixChannelFloat => (
            WaveFormat::new(
                32,
                32,
                SampleType::Float,
                rate as usize,
                mix_channels.max(1),
                None,
            ),
            mix_channels.max(1),
            WasapiSample::F32,
            true,
        ),
        FormatAttempt::MixFormat => {
            let (sample, channels) = sample_kind(&mix_format)?;
            (mix_format.clone(), channels, sample, false)
        }
    };

    let mode = StreamMode::EventsShared {
        autoconvert,
        buffer_duration_hns: buffer_duration,
    };
    // 设备原生格式这一档再追加「RAW 流」请求：`AUDCLNT_STREAMOPTIONS_RAW`
    // 是 Windows 10 2004+ 提供的**官方绕开音效处理**的开关（等价于独占模式下
    // 的原始通路），语义上要求格式与端点原生格式一致。
    //
    // 为什么需要它：实测采集回来的音频与原句互相关只有 -0.08（几乎不相关）但
    // 频谱正常（语音频带 70%），即音效处理把波形改写了；「人能听懂、模型认不出」，
    // 中文尤甚（声调依赖精确基频轮廓）。
    //
    // 设 RAW 只是**请求**：设备不接受时 `initialize_client` 会失败，所以失败
    // 就退回不设 RAW 再试一次，绝不因此让通道连不上。
    let wants_raw = matches!(attempt, FormatAttempt::MixFormat);
    if wants_raw {
        let props = AudioClientProperties::new().set_option(StreamOption::Raw);
        // set_properties 失败不算错：个别驱动不支持 IAudioClient2，继续走普通路径。
        let _ = client.set_properties(props);
    }
    // 对 render 端点以 Capture 方向初始化即得到 loopback 流。
    let raw_accepted =
        if let Err(error) = client.initialize_client(&format, Direction::Capture, mode, None) {
            if !wants_raw {
                return Err(format!("初始化 loopback 客户端失败：{error}"));
            }
            // RAW 被拒绝：退回普通属性重试一次。
            let _ = client.set_properties(AudioClientProperties::new());
            client
                .initialize_client(&format, Direction::Capture, mode, None)
                .map_err(|e| format!("初始化 loopback 客户端失败（已回退 RAW）：{e}"))?;
            false
        } else {
            wants_raw
        };

    let event = client
        .set_get_eventhandle()
        .map_err(|e| format!("创建事件句柄失败：{e}"))?;
    let capture = client
        .get_audiocaptureclient()
        .map_err(|e| format!("获取音频捕获客户端失败：{e}"))?;
    client
        .start_stream()
        .map_err(|e| format!("启动 loopback 流失败：{e}"))?;

    Ok(LoopbackParts {
        client,
        capture,
        event,
        rate,
        channels,
        sample,
        raw_accepted,
    })
}

/// 从设备混音格式推导样本编码与声道数。
fn sample_kind(format: &WaveFormat) -> std::result::Result<(WasapiSample, usize), String> {
    let channels = format.get_nchannels() as usize;
    if channels == 0 {
        return Err("设备声道数为 0".to_string());
    }
    let storage_bits = (format.get_blockalign() as usize * 8) / channels;
    let subformat = format
        .get_subformat()
        .map_err(|e| format!("不支持的音频子格式：{e}"))?;
    let sample = match (subformat, storage_bits) {
        (SampleType::Float, 32) => WasapiSample::F32,
        (SampleType::Float, 64) => WasapiSample::F64,
        (SampleType::Int, 16) => WasapiSample::I16,
        (SampleType::Int, 32) => WasapiSample::I32,
        (_, bits) => {
            return Err(format!("不支持的混音格式位深：{bits} bit"));
        }
    };
    Ok((sample, channels))
}

/// WASAPI 原始字节（本机小端）→ 单声道 f32，按帧求平均。
fn bytes_to_mono(
    bytes: &[u8],
    channels: usize,
    kind: WasapiSample,
    out: &mut Vec<f32>,
) -> std::result::Result<(), String> {
    out.clear();
    if channels == 0 {
        return Err("声道数为 0".to_string());
    }
    let bytes_per_sample = kind.bytes();
    let frame_bytes = bytes_per_sample
        .checked_mul(channels)
        .ok_or_else(|| "声道数过大".to_string())?;
    if frame_bytes == 0 {
        return Ok(());
    }

    out.reserve(bytes.len() / frame_bytes);
    let scale = 1.0 / channels as f32;
    for frame in bytes.chunks_exact(frame_bytes) {
        let mut sum = 0.0f32;
        for channel in 0..channels {
            let start = channel * bytes_per_sample;
            let slice = &frame[start..start + bytes_per_sample];
            let value = match kind {
                WasapiSample::F32 => f32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]),
                WasapiSample::F64 => f64::from_le_bytes([
                    slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
                ]) as f32,
                WasapiSample::I16 => i16::from_le_bytes([slice[0], slice[1]]) as f32 / 32_768.0,
                WasapiSample::I32 => {
                    i32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]) as f32
                        / 2_147_483_648.0
                }
            };
            sum += value;
        }
        out.push(sum * scale);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_to_mono_converts_stereo_float() {
        let mut bytes = Vec::new();
        for value in [1.0f32, -1.0, 0.5, 0.5] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let mut out = Vec::new();
        bytes_to_mono(&bytes, 2, WasapiSample::F32, &mut out).expect("转换失败");
        assert_eq!(out.len(), 2);
        assert!(out[0].abs() < 1e-6);
        assert!((out[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn bytes_to_mono_converts_i16_frames() {
        let mut bytes = Vec::new();
        for value in [16_384i16, 16_384] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let mut out = Vec::new();
        bytes_to_mono(&bytes, 2, WasapiSample::I16, &mut out).expect("转换失败");
        assert_eq!(out.len(), 1);
        assert!((out[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn bytes_to_mono_tolerates_partial_frame() {
        let bytes = [0u8; 6]; // 单声道 f32 需要 4 字节，6 字节应只产出 1 帧
        let mut out = Vec::new();
        bytes_to_mono(&bytes, 1, WasapiSample::F32, &mut out).expect("转换失败");
        assert_eq!(out.len(), 1);
    }
}
