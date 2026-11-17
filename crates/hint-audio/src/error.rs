//! 音频模块统一错误类型。
//!
//! 采集线程里的运行时错误不会以 `Result` 返回给调用方，而是通过
//! `on_error` 回调上报；这里的错误用于同步 API（设备枚举、引擎启动、
//! 重采样器构造等）。

use thiserror::Error;

/// 音频模块错误。
#[derive(Debug, Error)]
pub enum AudioError {
    /// 配置不合法（采样率、分块长度等）。
    #[error("音频配置无效：{0}")]
    InvalidConfig(String),
    /// 设备不存在、被占用或初始化失败。
    #[error("音频设备不可用：{0}")]
    Device(String),
    /// 音频后端（cpal / WASAPI）返回错误。
    #[error("音频后端错误：{0}")]
    Backend(String),
    /// 平台或格式不受支持。
    #[error("不支持：{0}")]
    Unsupported(String),
    /// 重采样过程失败。
    #[error("重采样失败：{0}")]
    Resample(String),
}

/// 本 crate 使用的 `Result` 别名。
pub type Result<T> = std::result::Result<T, AudioError>;
