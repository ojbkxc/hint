pub mod audio;
pub mod capture;
pub mod diagnostics;
pub mod error;
pub mod import;
pub mod llm;
pub mod pipeline;
pub mod proxy;
pub mod secret;
pub mod session;
pub mod settings;
pub mod stt;

pub use error::{CoreError, Result};
pub use llm::{ChatRequest, ChatResponse};
pub use settings::{AnswerStyle, ProviderKind, Settings, ThemeKind, TriggerMode};
