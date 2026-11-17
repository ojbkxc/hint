//! worker 与父进程之间共享的线上类型。
//!
//! 线上字段名用 camelCase，与 cc-haha 的协议保持一致；Rust 字段名沿用 snake_case。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::assets;
use crate::error::{Result, SttError};

/// SenseVoice 支持的语言提示。`auto` 由模型自行判别。
pub const WORKER_LANGUAGES: &[&str] = &["auto", "zh", "en", "ja", "ko", "yue"];

/// worker 启动时读取的配置环境变量。worker 读完立即删除。
pub const WORKER_CONFIG_ENV: &str = "HINT_VOICE_WORKER_CONFIG";

/// worker 启动时读取的 Bearer token 环境变量。worker 读完立即删除。
pub const WORKER_TOKEN_ENV: &str = "HINT_VOICE_WORKER_TOKEN";

/// 单次请求的录音上限：120 秒。
pub const MAX_AUDIO_SECONDS: f32 = 120.0;

/// 单次请求的 WAV 上限：4MiB。
pub const MAX_AUDIO_BYTES: u64 = 4 * 1024 * 1024;

/// 16k 单声道下的采样点数上限。
pub const MAX_AUDIO_SAMPLES: usize = (MAX_AUDIO_SECONDS * 16_000.0) as usize;

/// 一次识别的结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    /// 识别文本；各 VAD 段用单个空格拼接。
    pub text: String,
    /// 录音时长（秒）。
    pub audio_seconds: f32,
    /// 推理耗时（秒），不含 HTTP 往返。
    pub inference_seconds: f32,
}

/// 线上别名：worker 与 client 共用同一结构。
pub type WorkerTranscript = Transcript;

/// worker 的错误响应体。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerErrorBody {
    pub error: String,
    /// `Some("invalid-input")` 表示请求在推理前就被拒绝。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl WorkerErrorBody {
    /// 普通错误。
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            error: message.into(),
            code: None,
        }
    }

    /// 请求不合法（400/413）。
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            error: message.into(),
            code: Some("invalid-input".to_string()),
        }
    }
}

/// sidecar worker 的启动配置，序列化后放进 `HINT_VOICE_WORKER_CONFIG`。
///
/// 路径字段是绝对路径；`ModelManager` 负责把它们拼成 `WorkerConfig`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerConfig {
    /// `model.int8.onnx` 的绝对路径。
    pub model_path: PathBuf,
    /// `tokens.txt` 的绝对路径。
    pub tokens_path: PathBuf,
    /// `silero_vad.onnx` 的绝对路径。
    pub vad_path: PathBuf,
    /// 推理线程数，1..=16。
    #[serde(default = "default_threads")]
    pub threads: i32,
    /// 单次请求体上限（字节）。
    #[serde(default = "default_max_audio_bytes")]
    pub max_audio_bytes: u64,
    /// 单次请求录音时长上限（秒）。
    #[serde(default = "default_max_audio_seconds")]
    pub max_audio_seconds: f32,
    /// Silero VAD 阈值。
    #[serde(default = "default_vad_threshold")]
    pub vad_threshold: f32,
    /// 最短静音（秒），决定断句。
    #[serde(default = "default_min_silence_seconds")]
    pub min_silence_seconds: f32,
    /// 最短语音（秒），短于此的噪声丢弃。
    #[serde(default = "default_min_speech_seconds")]
    pub min_speech_seconds: f32,
    /// VAD 软上限（秒）。SenseVoice 没有 KV cache 上限，30s 足够。
    #[serde(default = "default_max_speech_seconds")]
    pub max_speech_seconds: f32,
}

fn default_threads() -> i32 {
    2
}
fn default_max_audio_bytes() -> u64 {
    MAX_AUDIO_BYTES
}
fn default_max_audio_seconds() -> f32 {
    MAX_AUDIO_SECONDS
}
fn default_vad_threshold() -> f32 {
    0.5
}
fn default_min_silence_seconds() -> f32 {
    0.5
}
fn default_min_speech_seconds() -> f32 {
    0.25
}
fn default_max_speech_seconds() -> f32 {
    30.0
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            model_path: PathBuf::new(),
            tokens_path: PathBuf::new(),
            vad_path: PathBuf::new(),
            threads: default_threads(),
            max_audio_bytes: default_max_audio_bytes(),
            max_audio_seconds: default_max_audio_seconds(),
            vad_threshold: default_vad_threshold(),
            min_silence_seconds: default_min_silence_seconds(),
            min_speech_seconds: default_min_speech_seconds(),
            max_speech_seconds: default_max_speech_seconds(),
        }
    }
}

impl WorkerConfig {
    /// 按模型目录（`<data_root>/sensevoice/`）生成标准配置。
    pub fn for_model_dir(model_dir: &Path) -> Self {
        Self {
            model_path: model_dir.join(assets::MODEL_PATH),
            tokens_path: model_dir.join(assets::TOKENS_PATH),
            vad_path: model_dir.join(assets::VAD_PATH),
            ..Self::default()
        }
    }

    /// worker 启动前校验：路径非空、线程数与 VAD 参数在合理区间。
    pub fn validate(&self) -> Result<()> {
        for (label, path) in [
            ("model_path", &self.model_path),
            ("tokens_path", &self.tokens_path),
            ("vad_path", &self.vad_path),
        ] {
            if path.as_os_str().is_empty() {
                return Err(SttError::Config {
                    message: format!("{label} is empty"),
                });
            }
        }
        if !(1..=16).contains(&self.threads) {
            return Err(SttError::Config {
                message: format!("threads out of range: {}", self.threads),
            });
        }
        if self.max_audio_bytes < 46 || self.max_audio_bytes > 64 * 1024 * 1024 {
            return Err(SttError::Config {
                message: format!("max_audio_bytes out of range: {}", self.max_audio_bytes),
            });
        }
        if !(1.0..=600.0).contains(&self.max_audio_seconds) {
            return Err(SttError::Config {
                message: format!("max_audio_seconds out of range: {}", self.max_audio_seconds),
            });
        }
        if !(0.0..=1.0).contains(&self.vad_threshold) {
            return Err(SttError::Config {
                message: format!("vad_threshold out of range: {}", self.vad_threshold),
            });
        }
        if self.min_silence_seconds <= 0.0 || self.min_silence_seconds > 10.0 {
            return Err(SttError::Config {
                message: format!(
                    "min_silence_seconds out of range: {}",
                    self.min_silence_seconds
                ),
            });
        }
        if self.min_speech_seconds < 0.0 || self.min_speech_seconds > 10.0 {
            return Err(SttError::Config {
                message: format!(
                    "min_speech_seconds out of range: {}",
                    self.min_speech_seconds
                ),
            });
        }
        if self.max_speech_seconds <= 0.0 || self.max_speech_seconds > 600.0 {
            return Err(SttError::Config {
                message: format!(
                    "max_speech_seconds out of range: {}",
                    self.max_speech_seconds
                ),
            });
        }
        Ok(())
    }

    /// VAD 内部缓冲区秒数：软上限 + 最短静音 + 1 秒余量。
    pub fn vad_buffer_seconds(&self) -> f32 {
        self.max_speech_seconds + self.min_silence_seconds + 1.0
    }
}

/// 校验语言提示是否在支持列表内。
pub fn validate_language(language: &str) -> Result<()> {
    if WORKER_LANGUAGES.contains(&language) {
        Ok(())
    } else {
        Err(SttError::InvalidInput {
            message: format!("unsupported speech language: {language}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_the_reference_tuning() {
        let config = WorkerConfig::default();
        assert_eq!(config.threads, 2);
        assert_eq!(config.max_audio_bytes, 4 * 1024 * 1024);
        assert_eq!(config.max_audio_seconds, 120.0);
        assert_eq!(config.vad_threshold, 0.5);
        assert_eq!(config.min_silence_seconds, 0.5);
        assert_eq!(config.min_speech_seconds, 0.25);
        assert_eq!(config.max_speech_seconds, 30.0);
        assert!((config.vad_buffer_seconds() - 31.5).abs() < f32::EPSILON);
    }

    #[test]
    fn config_for_model_dir_points_at_the_manifest_paths() {
        let config = WorkerConfig::for_model_dir(Path::new("C:\\data\\sensevoice"));
        assert!(config.model_path.ends_with("models\\model.int8.onnx"));
        assert!(config.tokens_path.ends_with("models\\tokens.txt"));
        assert!(config.vad_path.ends_with("models\\silero_vad.onnx"));
    }

    #[test]
    fn validate_rejects_bad_values() {
        let mut config = WorkerConfig::default();
        assert!(config.validate().is_err(), "empty paths must fail");
        config.model_path = PathBuf::from("m");
        config.tokens_path = PathBuf::from("t");
        config.vad_path = PathBuf::from("v");
        assert!(config.validate().is_ok());
        config.threads = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn language_whitelist_matches_protocol() {
        for language in WORKER_LANGUAGES {
            assert!(validate_language(language).is_ok());
        }
        assert!(validate_language("fr").is_err());
        assert!(validate_language("").is_err());
    }

    #[test]
    fn transcript_serializes_camel_case() {
        let value = serde_json::to_value(Transcript {
            text: "你好".into(),
            audio_seconds: 1.5,
            inference_seconds: 0.25,
        })
        .unwrap();
        assert_eq!(value["audioSeconds"], 1.5);
        assert_eq!(value["inferenceSeconds"], 0.25);
        assert_eq!(value["text"], "你好");
    }
}
