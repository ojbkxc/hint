//! 音频层纯逻辑：双通道联合状态机、噪声门、带迟滞的语音边缘检测。
pub mod joint_state;
pub mod noise_gate;

pub use joint_state::{JointChannel, JointSpeechState, JointSpeechTracker, JointSpeechTransition};
pub use noise_gate::{apply_noise_gate, audio_metrics, SpeechEdgeDetector, SPEECH_RMS};
