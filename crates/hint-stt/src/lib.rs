//! 本地语音识别：模型下载、sidecar worker、SenseVoice 推理。
//!
//! 模块分层：
//! - [`assets`]：编译期固化的资产清单（地址 / 字节数 / sha256）；
//! - [`manifest`]：`verified.json` 读写与就绪判断（只查大小，不重算哈希）；
//! - [`download`]：`.part` 断点续传 + 每源重试 + 边下边算 sha256；
//! - [`worker`]：sidecar 推理与 loopback HTTP（`engine` feature）；
//! - [`client`]：父进程侧 sidecar 管理（自唤 `--voice-worker`）；
//! - [`wav`] / [`types`]：WAV 解析与线上协议类型。
//!
//! 真实 sherpa-onnx 推理由 `engine` feature 控制；没有 C 工具链的机器可以用
//! `cargo check -p hint-stt --no-default-features` 只检查平台无关部分。

pub mod assets;
pub mod client;
pub mod download;
pub mod error;
pub mod manifest;
pub mod types;
pub mod wav;

#[cfg(feature = "engine")]
pub mod worker;

pub use assets::{MODEL_ID, RemoteFile, TOTAL_BYTES};
pub use client::{WORKER_FLAG, WorkerClient};
pub use download::{DownloadProgress, DownloadStep, ModelManager, ModelStatus};
pub use error::{FailureReason, Result, SttError};
pub use types::{
    MAX_AUDIO_BYTES, MAX_AUDIO_SECONDS, Transcript, WORKER_CONFIG_ENV, WORKER_TOKEN_ENV,
    WorkerConfig, WorkerTranscript,
};

#[cfg(feature = "engine")]
pub use worker::run_worker_from_env;

/// 未启用 `engine` feature 时的占位实现：让集成方的 `main.rs` 在任何 feature
/// 组合下都能编译，实际运行会明确报「不支持」。
#[cfg(not(feature = "engine"))]
pub fn run_worker_from_env() -> Result<()> {
    Err(SttError::Unsupported {
        message: "hint-stt was built without the `engine` feature".into(),
    })
}
