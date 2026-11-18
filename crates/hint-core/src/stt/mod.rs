//! STT 鲁棒性纯逻辑：环形缓冲、断线跟踪、片段切分、补录去重。
pub mod clips;
pub mod dedup;
pub mod gap;
pub mod ring;

pub use clips::{split_into_speech_clips, ClipOptions, SpeechClip};
pub use dedup::is_covered_by_live;
pub use gap::{stall_backoff_ms, Outage, OutageReason, SttGapTracker};
pub use ring::{PcmFrame, PcmRingBuffer};
