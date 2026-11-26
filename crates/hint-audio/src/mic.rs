//! 麦克风采集线程（cpal）。
//!
//! 结构：
//! - 「hint-mic」线程负责打开设备与输入流，并在循环中消费样本队列；
//! - cpal 数据回调只做「交错 → 单声道 f32 降混」并推入有界队列，
//!   不在音频线程里做重采样；
//! - 流错误（设备拔出、路由失效）通过错误回调上报；线程按退避策略
//!   重新打开流，连续失败 5 次则判定为致命并停止整个引擎。
//!
//! 指定设备不存在时回退默认输入设备，并通过 `on_error` 发出警告。

use crate::Channel;
use crate::pipeline::ChannelPipeline;
use crate::queue::SampleQueue;
use crate::runtime::Runtime;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Data, FromSample, Sample, SampleFormat, SizedSample};
use std::sync::Arc;
use std::sync::mpsc::{self, Sender, TryRecvError};
use std::time::{Duration, Instant};

/// 连续失败多少次后判定通道死亡。
const MAX_CONSECUTIVE_FAILURES: u32 = 5;
/// 失败至少要持续这么久才判致命：用户在系统设置里切设备（蓝牙耳机、USB 麦）
/// 常出现数秒的短暂失败，不应因此终止整场面试。真正拔掉的设备仍会在宽限后停止。
const FATAL_GRACE: Duration = Duration::from_secs(15);
/// 流稳定运行多久后清零失败计数。
const HEALTHY_AFTER: Duration = Duration::from_secs(3);

/// 连续失败超过致命阈值后，转入低频重试的间隔。
///
/// 为什么不是永久停止：麦克风被会议软件抢占后，对方释放时应该能自己恢复；
/// 为什么不保持高频：设备真的不存在时会白烧 CPU 并刷满日志。
const LOW_FREQ_RETRY_INTERVAL: Duration = Duration::from_secs(10);

/// 麦克风「曾经打开成功、之后又失败」时的排查提示。
///
/// 为什么要区分两种失败：真机日志（用户机器）显示
///
/// ```text
/// 06:12:19  麦克风采集已启动：48000 Hz
/// 06:14:07  麦克风采集中断：拒绝访问 (os error -2147024891)
/// ```
///
/// **先能用、约两分钟后失去访问**——这不是隐私权限没开（没开的话第一次就打不开），
/// 而是麦克风被别的应用抢占了：会议软件（腾讯会议/钉钉）在入会、切设备或检测到
/// 通话时会以独占模式拿麦，蓝牙耳机切到 HFP（通话）模式也会这样。
///
/// 原文案只说「请检查麦克风设备与隐私权限」，把人往错误方向引——用户去翻 Windows
/// 隐私设置会发现麦克风明明是允许的。
fn lost_access_hint(was_running: bool) -> &'static str {
    if was_running {
        "（麦克风原本可用、之后被系统拒绝：常见原因是会议软件以独占模式占用了麦克风，\
         或蓝牙耳机切换到了通话模式。关闭占用麦克风的应用后重新开始会话即可，\
         不需要改 Windows 隐私权限。）"
    } else {
        "（麦克风从未成功打开：请检查 Windows 设置 → 隐私和安全性 → 麦克风是否允许\
         桌面应用访问，以及是否选了不存在的设备。）"
    }
}
/// 队列容量（样本数）：约 10 秒 @48 kHz 单声道；溢出丢弃最旧数据。
const QUEUE_CAPACITY_SAMPLES: usize = 480_000;
/// 队列消费等待超时，决定停止响应速度。
const QUEUE_WAIT: Duration = Duration::from_millis(50);

/// 麦克风采集线程入口。
pub(crate) fn run(rt: Arc<Runtime>, init: Sender<std::result::Result<(), String>>) {
    let mut init = Some(init);
    let mut failures: u32 = 0;
    // 本轮连续失败的起始时刻；成功稳定运行或判致命后清空。
    let mut first_failure_at: Option<Instant> = None;
    // 已转入低频重试：只在首次进入时提示一次，避免每 10 秒刷一条同样的警告。
    let mut low_freq_retry_active = false;

    loop {
        if rt.should_stop() {
            break;
        }

        let queue = Arc::new(SampleQueue::new(QUEUE_CAPACITY_SAMPLES));
        let (err_tx, err_rx) = mpsc::channel::<String>();

        match build_stream(&rt, &queue, &err_tx) {
            Ok((stream, input_rate, input_channels)) => {
                let mut pipeline = match ChannelPipeline::new(
                    Channel::Me,
                    input_rate,
                    rt.config.target_sample_rate,
                    rt.config.chunk_samples,
                ) {
                    Ok(pipeline) => pipeline,
                    Err(error) => {
                        // 连续失败已超过宽限期仍建不起来（例如设备在重建后反复报出
                        // 非法采样率），才判致命；否则仅上报并继续退避重试。
                        if failure_grace_exceeded(first_failure_at) {
                            let message = format!("麦克风重采样初始化失败，采集引擎停止：{error}");
                            if let Some(tx) = init.take() {
                                let _ = tx.send(Err(message));
                            } else {
                                rt.error(&message);
                            }
                            rt.fatal();
                            break;
                        }
                        let message = format!("麦克风重采样初始化失败：{error}");
                        if let Some(tx) = init.take() {
                            let _ = tx.send(Err(message));
                            break;
                        }
                        rt.error(format!("麦克风重采样初始化失败：{error}，将重试"));
                        failures = failures.saturating_add(1);
                        first_failure_at.get_or_insert_with(Instant::now);
                        queue.close();
                        drop(stream);
                        rt.sleep_backoff(failures);
                        continue;
                    }
                };

                if let Some(tx) = init.take() {
                    let _ = tx.send(Ok(()));
                }
                tracing::info!(
                    "麦克风采集已启动：输入 {} Hz，声道 {}，目标 {} Hz",
                    input_rate,
                    input_channels,
                    rt.config.target_sample_rate
                );

                let started = Instant::now();
                let mut healthy = false;
                let mut batch: Vec<f32> = Vec::with_capacity(8192);
                let mut failure_reason: Option<String> = None;

                loop {
                    if rt.should_stop() {
                        break;
                    }
                    if !healthy && started.elapsed() >= HEALTHY_AFTER {
                        failures = 0;
                        first_failure_at = None;
                        healthy = true;
                        // 恢复稳定运行：下次再失败要从「正常阈值」重新计数，
                        // 不能因为上次进过低频重试就一直低频。
                        if low_freq_retry_active {
                            low_freq_retry_active = false;
                            tracing::info!("麦克风采集已恢复（此前处于低频重试）");
                        }
                    }

                    let open = queue.take(QUEUE_WAIT, &mut batch);
                    if !batch.is_empty() {
                        match pipeline.push_mono(&batch) {
                            Ok(chunks) => {
                                for chunk in chunks {
                                    rt.publish_chunk(chunk);
                                }
                            }
                            Err(error) => {
                                failure_reason = Some(format!("重采样失败：{error}"));
                                break;
                            }
                        }
                    }
                    if !open {
                        break;
                    }
                    match err_rx.try_recv() {
                        Ok(message) => {
                            failure_reason = Some(message);
                            break;
                        }
                        Err(TryRecvError::Empty) => {}
                        Err(TryRecvError::Disconnected) => break,
                    }
                }

                // 显式关闭队列并丢弃流，确保回调停止后再进入下一轮。
                queue.close();
                drop(stream);

                match failure_reason {
                    Some(reason) => {
                        rt.error(format!(
                            "麦克风采集中断：{reason}{}",
                            lost_access_hint(init.is_none())
                        ));
                        failures = failures.saturating_add(1);
                        first_failure_at.get_or_insert_with(Instant::now);
                    }
                    None => break, // 正常停止
                }
            }
            Err(message) => {
                if let Some(tx) = init.take() {
                    // 首次启动失败：交给 AudioEngine::start 统一上报并返回错误。
                    // 这里也带提示——「从未打开成功」和「运行中失去」的排查方向
                    // 完全不同，混成一句会把用户引到错误的地方。
                    let _ = tx.send(Err(format!("{message}{}", lost_access_hint(false))));
                    break;
                }
                rt.error(format!(
                    "麦克风设备重新打开失败：{message}{}",
                    lost_access_hint(true)
                ));
                failures = failures.saturating_add(1);
                first_failure_at.get_or_insert_with(Instant::now);
            }
        }

        if rt.should_stop() {
            break;
        }
        if failures >= MAX_CONSECUTIVE_FAILURES && failure_grace_exceeded(first_failure_at) {
            // 不直接判死，转成**低频重试**。
            //
            // 真机日志（用户机器）：麦克风用了约两分钟后「拒绝访问」，连续 5 次失败后
            // 引擎停止——用户必须手动结束再开始会话才能恢复。而那台机器上是会议软件
            // 抢占了麦克风：等它释放（几秒到几分钟）本该能自己恢复。
            //
            // 永久停止的代价太大：面试进行中丢麦，用户根本来不及去点「结束/开始」，
            // 整场面试就废了。而无限高频重试的代价是白烧 CPU 与刷日志——用低频重试
            // 折中：每 LOW_FREQ_RETRY_INTERVAL 试一次，恢复后自动继续采集。
            //
            // 只有「设备真的不存在」这类永久错误才会一直失败；那种情况下用户本来也
            // 需要换设备，低频重试只是让日志安静些。
            if !low_freq_retry_active {
                low_freq_retry_active = true;
                rt.error(format!(
                    "麦克风通道连续 {MAX_CONSECUTIVE_FAILURES} 次失败，转为每 {} 秒重试一次\
                     （恢复占用后会自动继续采集，不需要重开会话）",
                    LOW_FREQ_RETRY_INTERVAL.as_secs()
                ));
            }
            rt.finish_channel(Channel::Me);
            // 低频等待期间仍要响应停止请求，所以按小片睡。
            let deadline = Instant::now() + LOW_FREQ_RETRY_INTERVAL;
            while Instant::now() < deadline && !rt.should_stop() {
                rt.sleep_backoff(MAX_CONSECUTIVE_FAILURES);
            }
            if rt.should_stop() {
                break;
            }
            continue;
        }
        // 断线期间该通道没有音频：先清电平、重置迟滞检测器并把联合状态标为
        // 静默。否则断开前若正在说话，重连后同向边会被状态机吸收，界面会
        // 一直停在「你在说」。
        rt.finish_channel(Channel::Me);
        rt.sleep_backoff(failures);
    }

    rt.finish_channel(Channel::Me);
}

/// 连续失败是否已持续超过 `FATAL_GRACE`。
fn failure_grace_exceeded(first_failure_at: Option<Instant>) -> bool {
    first_failure_at.is_some_and(|at| at.elapsed() >= FATAL_GRACE)
}

/// 打开输入流；返回流对象、设备原生采样率与声道数。
fn build_stream(
    rt: &Runtime,
    queue: &Arc<SampleQueue>,
    err_tx: &Sender<String>,
) -> std::result::Result<(cpal::Stream, u32, usize), String> {
    let host = cpal::default_host();
    let device = resolve_device(&host, rt.config.mic_device.as_deref(), rt)?;
    let supported = device
        .default_input_config()
        .map_err(|e| format!("读取麦克风默认格式失败：{e}"))?;
    let sample_format = supported.sample_format();
    let config = supported.config();
    let channels = config.channels as usize;
    if channels == 0 {
        return Err("麦克风声道数为 0".to_string());
    }
    let input_rate = config.sample_rate;

    let queue = queue.clone();
    let data_errors = err_tx.clone();
    let stream_errors = err_tx.clone();
    let mut scratch: Vec<f32> = Vec::with_capacity(8192);

    let stream = device
        .build_input_stream_raw(
            config,
            sample_format,
            move |data: &Data, _info| match convert_into(data, channels, &mut scratch) {
                Ok(()) => queue.push(&scratch),
                Err(error) => {
                    // 回调线程只上报，不 panic；处理线程会看到错误并重连。
                    let _ = data_errors.send(error);
                }
            },
            move |error| {
                // 默认设备切换时 CPAL 已自动把流迁移到新设备，无需重建。
                if error.kind() == cpal::ErrorKind::DeviceChanged {
                    tracing::debug!("麦克风设备已切换，流已自动跟随：{error}");
                    return;
                }
                // WASAPI 的瞬时缓冲欠载/过载（"A buffer underrun or overrun
                // occurred."）在真机上高频出现且流仍然存活：cpal 在回调里
                // 报这个错只是提示丢了一小段，不代表设备失效。把它当通道
                // 失败走「重建流」会造成重连风暴（实测 41 次 underrun 对应
                // 43 次全量重开流，会话因 5 连失败超 15 秒宽限被整体判死）。
                // 这里降为 debug：流不死就继续跑，下一帧数据照常到达。
                // WASAPI 实际以 `ErrorKind::Xrun` 上报（cpal 0.18 wasapi/stream.rs
                // 在 GetBuffer 带 DATA_DISCONTINUITY 标志时 `ErrorKind::Xrun.into()`），
                // 部分后端则退化为 `Other` + 文本。两种都要识别，否则判定条件落空、
                // 欠载继续被当成通道失败而重建流。
                let kind = error.kind();
                let xrun_text = error
                    .to_string()
                    .to_ascii_lowercase()
                    .contains("buffer underrun or overrun");
                if kind == cpal::ErrorKind::Xrun || (kind == cpal::ErrorKind::Other && xrun_text) {
                    tracing::debug!("麦克风缓冲瞬时欠载（流仍在运行，忽略）：{error}");
                    return;
                }
                let _ = stream_errors.send(format!("{error}"));
            },
            Some(Duration::from_secs(10)),
        )
        .map_err(|e| format!("创建麦克风输入流失败：{e}"))?;

    stream
        .play()
        .map_err(|e| format!("启动麦克风输入流失败：{e}"))?;
    Ok((stream, input_rate, channels))
}

/// 按 ID（cpal DeviceId 字符串）、原始 ID 或名称解析输入设备；失败回退默认设备。
fn resolve_device(
    host: &cpal::Host,
    wanted: Option<&str>,
    rt: &Runtime,
) -> std::result::Result<cpal::Device, String> {
    if let Some(wanted) = wanted.filter(|value| !value.is_empty()) {
        // `device_by_id` 的默认实现遍历全部端点（含 render），若用户保存的 ID
        // 恰好指向输出设备，随后 `default_input_config()` 会以晦涩错误失败。
        // 这里先校验方向，方向不符就当作「未找到」回退默认输入设备。
        if let Ok(parsed) = wanted.parse::<cpal::DeviceId>()
            && let Some(device) = host.device_by_id(&parsed)
            && is_input_device(&device)
        {
            return Ok(device);
        }
        if let Ok(devices) = host.input_devices() {
            for device in devices {
                let matches_id = device
                    .id()
                    .map(|id| id.to_string() == wanted)
                    .unwrap_or(false);
                let matches_name = device
                    .description()
                    .map(|description| description.name() == wanted)
                    .unwrap_or(false);
                if matches_id || matches_name {
                    return Ok(device);
                }
            }
        }
        rt.warning(format!(
            "未找到指定的麦克风「{wanted}」，已回退到默认输入设备"
        ));
    }
    host.default_input_device()
        .ok_or_else(|| "未找到可用的默认麦克风，请检查系统隐私设置中的麦克风权限".to_string())
}

/// 设备是否支持音频输入（方向为 Input/Duplex，或描述不可用时按允许处理）。
fn is_input_device(device: &cpal::Device) -> bool {
    match device.description() {
        Ok(description) => description.supports_input(),
        Err(_) => true,
    }
}

/// cpal 动态采样格式 → 单声道 f32。
fn convert_into(
    data: &Data,
    channels: usize,
    out: &mut Vec<f32>,
) -> std::result::Result<(), String> {
    macro_rules! convert {
        ($ty:ty) => {{
            let slice = data.as_slice::<$ty>().ok_or_else(|| {
                format!(
                    "麦克风采样数据与格式 {:?} 不一致",
                    data.sample_format()
                )
            })?;
            downmix_typed(slice, channels, out);
            Ok(())
        }};
    }

    match data.sample_format() {
        SampleFormat::F32 => convert!(f32),
        SampleFormat::F64 => convert!(f64),
        SampleFormat::I8 => convert!(i8),
        SampleFormat::I16 => convert!(i16),
        SampleFormat::I24 => convert!(cpal::I24),
        SampleFormat::I32 => convert!(i32),
        SampleFormat::I64 => convert!(i64),
        SampleFormat::U8 => convert!(u8),
        SampleFormat::U16 => convert!(u16),
        SampleFormat::U24 => convert!(cpal::U24),
        SampleFormat::U32 => convert!(u32),
        SampleFormat::U64 => convert!(u64),
        other => Err(format!("不支持的麦克风采样格式：{other}")),
    }
}

/// 交错样本 → 单声道 f32，按帧求平均。
fn downmix_typed<T>(input: &[T], channels: usize, out: &mut Vec<f32>)
where
    T: Sample + SizedSample + Copy,
    f32: FromSample<T>,
{
    out.clear();
    if channels == 0 || input.is_empty() {
        return;
    }
    let frames = input.len() / channels;
    out.reserve(frames);
    let scale = 1.0 / channels as f32;
    for frame in input.chunks_exact(channels) {
        let mut sum = 0.0f32;
        for &sample in frame {
            sum += f32::from_sample(sample);
        }
        out.push(sum * scale);
    }
}
