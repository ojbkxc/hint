//! 双通道音频引擎的启动与线程编排（Windows）。
//!
//! 每个通道一个后台线程：
//! - `hint-mic`：cpal 麦克风采集；
//! - `hint-system-audio`：WASAPI loopback 系统声采集。
//!
//! 启动时两个线程会通过握手通道回报初始化结果：两个通道都失败则
//! `start()` 返回错误；只有一个失败则通过 `on_error` 上报，引擎继续
//! 以单通道运行。

use crate::error::{AudioError, Result};
use crate::runtime::{AudioHandle, Runtime, Worker};
use crate::{AudioChunk, AudioConfig, JointSpeechTransition, Levels, mic, system};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

/// 等待通道初始化结果的上限。
///
/// 蓝牙/虚拟声卡在系统刚唤醒时打开可能需要 10 秒以上；10 秒会把「稍后
/// 成功」误报成通道失败。15 秒与「模型加载 10–30 秒」的启动预期匹配。
const INIT_WAIT: Duration = Duration::from_secs(15);

type InitResult = std::result::Result<(), String>;

pub(crate) fn start(
    config: AudioConfig,
    on_chunk: impl Fn(AudioChunk) + Send + Sync + 'static,
    on_levels: impl Fn(Levels) + Send + Sync + 'static,
    on_error: impl Fn(String) + Send + Sync + 'static,
    on_speech_edge: impl Fn(JointSpeechTransition) + Send + Sync + 'static,
) -> Result<AudioHandle> {
    let runtime = Arc::new(Runtime::new(
        config,
        on_chunk,
        on_levels,
        on_error,
        on_speech_edge,
    ));

    let (mic_init_tx, mic_init_rx) = mpsc::channel::<InitResult>();
    let (sys_init_tx, sys_init_rx) = mpsc::channel::<InitResult>();

    let mic_runtime = runtime.clone();
    let mic_exit_runtime = runtime.clone();
    let mic_join = thread::Builder::new()
        .name("hint-mic".to_string())
        .spawn(move || {
            // 依赖内部 panic 时也要让 is_running() 反映真实状态。
            let outcome = catch_unwind(AssertUnwindSafe(|| mic::run(mic_runtime, mic_init_tx)));
            if outcome.is_err() {
                mic_exit_runtime.error("麦克风采集线程异常退出，引擎停止");
                mic_exit_runtime.fatal();
            }
        })
        .map_err(|e| AudioError::Backend(format!("无法创建麦克风采集线程：{e}")))?;

    let sys_runtime = runtime.clone();
    let sys_exit_runtime = runtime.clone();
    let sys_join = match thread::Builder::new()
        .name("hint-system-audio".to_string())
        .spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| system::run(sys_runtime, sys_init_tx)));
            if outcome.is_err() {
                sys_exit_runtime.error("系统声采集线程异常退出，引擎停止");
                sys_exit_runtime.fatal();
            }
        }) {
        Ok(join) => join,
        Err(error) => {
            runtime.stop.store(true, Ordering::SeqCst);
            let _ = mic_join.join();
            return Err(AudioError::Backend(format!(
                "无法创建系统声采集线程：{error}"
            )));
        }
    };

    let workers = vec![
        Worker {
            name: "hint-mic",
            id: mic_join.thread().id(),
            handle: mic_join,
        },
        Worker {
            name: "hint-system-audio",
            id: sys_join.thread().id(),
            handle: sys_join,
        },
    ];

    let mic_result = mic_init_rx.recv_timeout(INIT_WAIT);
    let sys_result = sys_init_rx.recv_timeout(INIT_WAIT);
    let mic_ok = matches!(&mic_result, Ok(Ok(())));
    let sys_ok = matches!(&sys_result, Ok(Ok(())));

    if !mic_ok {
        runtime.error(match &mic_result {
            Ok(Err(message)) => format!("麦克风通道启动失败：{message}"),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                "麦克风通道异常退出（线程 panic）".to_string()
            }
            Err(mpsc::RecvTimeoutError::Timeout) => "麦克风通道初始化超时".to_string(),
            Ok(Ok(())) => "麦克风通道状态异常".to_string(),
        });
    }
    if !sys_ok {
        runtime.error(match &sys_result {
            Ok(Err(message)) => format!("系统声通道启动失败：{message}"),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                "系统声通道异常退出（线程 panic）".to_string()
            }
            Err(mpsc::RecvTimeoutError::Timeout) => "系统声通道初始化超时".to_string(),
            Ok(Ok(())) => "系统声通道状态异常".to_string(),
        });
    }

    // 单通道降级：引擎继续运行，但用户必须明确知道哪个通道没起来，
    // 否则「系统声没起来、听不到面试官」时会以为功能坏了。
    // 以「警告：」前缀发出，避免被上层当作致命错误处理。
    if mic_ok && !sys_ok {
        runtime.warning(
            "仅麦克风可用：系统声通道未能启动，将听不到对方的声音。\
             请检查系统输出设备后在设置页重新选择并重启会话",
        );
    } else if sys_ok && !mic_ok {
        runtime.warning(
            "仅系统声可用：麦克风通道未能启动，你的回答不会被识别。\
             如果刚才麦克风是正常的、突然变成这样，通常是别的应用（腾讯会议/钉钉等）\
             以独占模式占用了麦克风，或蓝牙耳机切到了通话模式——关掉占用它的应用后\
             重新开始会话即可。若是从一开始就打不开，请检查 Windows 设置 → 隐私和安全性\
             → 麦克风是否允许桌面应用访问，然后在设置页确认麦克风设备后重启会话。",
        );
    }

    if !mic_ok && !sys_ok {
        let handle = AudioHandle::new(runtime, workers);
        handle.stop();
        return Err(AudioError::Device(
            "麦克风与系统声通道均无法启动，请检查系统音频设备与麦克风隐私权限".to_string(),
        ));
    }

    Ok(AudioHandle::new(runtime, workers))
}
