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
use crate::{AudioChunk, AudioConfig, Levels, mic, system};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

/// 等待通道初始化结果的上限。
const INIT_WAIT: Duration = Duration::from_secs(10);

type InitResult = std::result::Result<(), String>;

pub(crate) fn start(
    config: AudioConfig,
    on_chunk: impl Fn(AudioChunk) + Send + Sync + 'static,
    on_levels: impl Fn(Levels) + Send + Sync + 'static,
    on_error: impl Fn(String) + Send + Sync + 'static,
) -> Result<AudioHandle> {
    let runtime = Arc::new(Runtime::new(config, on_chunk, on_levels, on_error));

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

    if !mic_ok && !sys_ok {
        let handle = AudioHandle::new(runtime, workers);
        handle.stop();
        return Err(AudioError::Device(
            "麦克风与系统声通道均无法启动，请检查系统音频设备与麦克风隐私权限".to_string(),
        ));
    }

    Ok(AudioHandle::new(runtime, workers))
}
