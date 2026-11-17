//! hint 的双通道音频采集库：麦克风（cpal）+ 系统声（Windows WASAPI loopback）。
//!
//! # 输出约定
//!
//! - 两个通道统一重采样为 `target_sample_rate`（默认 16 kHz）单声道 `f32`；
//! - 按 `chunk_samples`（默认 512，约 32 ms）切块，通过 `on_chunk` 回调；
//! - `Channel::Interviewer` = 系统声（面试官），`Channel::Me` = 麦克风（我）；
//! - `on_levels` 以约 30 fps 回调双通道 RMS 电平；
//! - 运行时错误（设备拔出、读取连续失败、重连失败）通过 `on_error` 上报，
//!   不 panic；任一通道连续失败 5 次后引擎明确停止，`is_running()` 变为
//!   `false`。
//!
//! # 平台
//!
//! Windows 提供完整双通道；其他平台可以编译、可以枚举设备，但
//! [`AudioEngine::start`] 返回 [`AudioError::Unsupported`]。
//!
//! # 线程模型
//!
//! 每个通道各有一个后台线程（`hint-mic` / `hint-system-audio`），回调
//! 从这两个线程触发；`on_chunk` / `on_levels` / `on_error` 必须是非阻塞、
//! 线程安全的。`AudioHandle::stop` 幂等，`AudioHandle` 被 `Drop` 时
//! 自动停止并回收线程。

mod devices;
#[cfg(windows)]
mod engine;
mod error;
mod level;
#[cfg(windows)]
mod mic;
mod pipeline;
mod queue;
mod resample;
mod runtime;
#[cfg(windows)]
mod system;

pub use devices::{list_input_devices, list_output_devices};
pub use error::{AudioError, Result};
pub use runtime::AudioHandle;

use serde::{Deserialize, Serialize};

/// 音频通道。
///
/// 序列化采用小写：`Interviewer` → `"interviewer"`，`Me` → `"me"`，
/// 与前端 / 存储层约定一致。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    /// 系统声通道（面试官）。
    Interviewer,
    /// 麦克风通道（自己）。
    Me,
}

impl Channel {
    /// 通道的中文名称，用于日志与错误提示。
    pub fn label(self) -> &'static str {
        match self {
            Channel::Interviewer => "系统声",
            Channel::Me => "麦克风",
        }
    }
}

/// 音频设备信息。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// 设备 ID（cpal `DeviceId` 字符串，形如 `wasapi:{端点ID}`）。
    pub id: String,
    /// 设备友好名称。
    pub name: String,
    /// 是否为系统默认设备（输入按默认麦克风、输出按默认播放设备判断）。
    pub is_default: bool,
}

/// 采集配置。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// 麦克风设备 ID；`None` 或空字符串表示默认输入设备。
    pub mic_device: Option<String>,
    /// 系统声输出设备 ID；`None` 或空字符串表示默认 render 端点。
    pub output_device: Option<String>,
    /// 目标采样率，默认 16000。
    pub target_sample_rate: u32,
    /// 每块样本数，默认 512（16 kHz 下约 32 ms）。
    pub chunk_samples: usize,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            mic_device: None,
            output_device: None,
            target_sample_rate: 16_000,
            chunk_samples: 512,
        }
    }
}

impl AudioConfig {
    fn validate(&self) -> Result<()> {
        if !(8_000..=192_000).contains(&self.target_sample_rate) {
            return Err(AudioError::InvalidConfig(format!(
                "target_sample_rate={} 超出 8000..=192000",
                self.target_sample_rate
            )));
        }
        if self.chunk_samples == 0 || self.chunk_samples > 65_536 {
            return Err(AudioError::InvalidConfig(format!(
                "chunk_samples={} 超出 1..=65536",
                self.chunk_samples
            )));
        }
        Ok(())
    }
}

/// 一个定长音频块。
#[derive(Clone, Debug)]
pub struct AudioChunk {
    /// 来源通道。
    pub channel: Channel,
    /// 单声道 f32 样本，长度固定为 `AudioConfig::chunk_samples`。
    pub samples: Vec<f32>,
    /// 采样率（等于 `AudioConfig::target_sample_rate`）。
    pub sample_rate: u32,
}

/// 双通道 RMS 电平。
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Levels {
    /// 麦克风（我）通道 RMS。
    pub mic_rms: f32,
    /// 系统声（面试官）通道 RMS。
    pub system_rms: f32,
}

/// 音频采集引擎入口（无状态）。
pub struct AudioEngine;

impl AudioEngine {
    /// 启动双通道采集。
    ///
    /// * `config`：设备与输出格式配置；
    /// * `on_chunk`：定长音频块回调（来自两个采集线程）；
    /// * `on_levels`：双通道电平回调（约 30 fps）；
    /// * `on_error`：运行时错误 / 警告回调（字符串为中文提示）。
    ///
    /// 返回的 [`AudioHandle`] 存活期间引擎持续工作；调用 `stop()` 或
    /// `Drop` 后停止。若两个通道都无法初始化，返回错误。
    pub fn start(
        config: AudioConfig,
        on_chunk: impl Fn(AudioChunk) + Send + Sync + 'static,
        on_levels: impl Fn(Levels) + Send + Sync + 'static,
        on_error: impl Fn(String) + Send + Sync + 'static,
    ) -> Result<AudioHandle> {
        config.validate()?;

        #[cfg(not(windows))]
        {
            let _ = (config, on_chunk, on_levels, on_error);
            return Err(AudioError::Unsupported(
                "系统声 loopback 仅支持 Windows；其他平台暂不可用".to_string(),
            ));
        }

        #[cfg(windows)]
        {
            engine::start(config, on_chunk, on_levels, on_error)
        }
    }
}

/// 忽略锁中毒的互斥锁辅助，保证采集线程不会因此 panic。
pub(crate) fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&Channel::Interviewer).unwrap(),
            "\"interviewer\""
        );
        assert_eq!(serde_json::to_string(&Channel::Me).unwrap(), "\"me\"");
    }

    #[test]
    fn handle_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<AudioHandle>();
        assert_send_sync::<DeviceInfo>();
        assert_send_sync::<AudioChunk>();
    }

    #[test]
    fn default_config_is_16k_512() {
        let config = AudioConfig::default();
        assert_eq!(config.target_sample_rate, 16_000);
        assert_eq!(config.chunk_samples, 512);
        assert!(config.mic_device.is_none());
        assert!(config.output_device.is_none());
    }

    #[test]
    fn invalid_config_is_rejected() {
        let config = AudioConfig {
            target_sample_rate: 0,
            ..AudioConfig::default()
        };
        assert!(AudioEngine::start(config, |_| {}, |_| {}, |_| {}).is_err());
        let config = AudioConfig {
            chunk_samples: 0,
            ..AudioConfig::default()
        };
        assert!(AudioEngine::start(config, |_| {}, |_| {}, |_| {}).is_err());
    }
}
