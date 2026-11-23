//! 引擎运行时状态与句柄。
//!
//! [`Runtime`] 由两个采集线程共享：停止标志、运行状态、回调集合与
//! 电平总线。`AudioHandle` 持有运行时与工作线程，`stop` 幂等，
//! `Drop` 时自动停止并回收线程。

use crate::level::LevelsBus;
use crate::lock;
use crate::{AudioChunk, AudioConfig, Channel, Levels};
use hint_core::audio::{
    JointChannel, JointSpeechTracker, JointSpeechTransition, SPEECH_RMS, SpeechEdgeDetector,
    apply_noise_gate,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{JoinHandle, ThreadId};
use std::time::{Duration, Instant};

/// 共享运行时状态。
pub(crate) struct Runtime {
    pub(crate) config: AudioConfig,
    /// 停止请求标志；`stop()` 或任一通道致命失败时置位。
    pub(crate) stop: AtomicBool,
    /// 引擎是否仍在工作（`is_running` 的真实来源）。
    pub(crate) running: AtomicBool,
    pub(crate) levels: LevelsBus,
    on_chunk: Box<dyn Fn(AudioChunk) + Send + Sync>,
    on_levels: Box<dyn Fn(Levels) + Send + Sync>,
    on_error: Box<dyn Fn(String) + Send + Sync>,
    on_speech_edge: Box<dyn Fn(JointSpeechTransition) + Send + Sync>,
    /// 每通道一个迟滞边检测器（索引 0=Interviewer，1=Me）。
    detectors: Mutex<[SpeechEdgeDetector; 2]>,
    /// 双通道联合语音状态机。
    joint: Mutex<JointSpeechTracker>,
    /// 引擎时间轴起点，用于计算 speech-edge 的 `at_ms`。
    epoch: Instant,
}

impl Runtime {
    pub(crate) fn new<C, L, E, S>(
        config: AudioConfig,
        on_chunk: C,
        on_levels: L,
        on_error: E,
        on_speech_edge: S,
    ) -> Self
    where
        C: Fn(AudioChunk) + Send + Sync + 'static,
        L: Fn(Levels) + Send + Sync + 'static,
        E: Fn(String) + Send + Sync + 'static,
        S: Fn(JointSpeechTransition) + Send + Sync + 'static,
    {
        Self {
            config,
            stop: AtomicBool::new(false),
            running: AtomicBool::new(true),
            levels: LevelsBus::new(),
            on_chunk: Box::new(on_chunk),
            on_levels: Box::new(on_levels),
            on_error: Box::new(on_error),
            on_speech_edge: Box::new(on_speech_edge),
            detectors: Mutex::new([
                SpeechEdgeDetector::new(SPEECH_RMS),
                SpeechEdgeDetector::new(SPEECH_RMS),
            ]),
            joint: Mutex::new(JointSpeechTracker::new()),
            epoch: Instant::now(),
        }
    }

    /// 是否已请求停止。
    pub(crate) fn should_stop(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    /// 上报非致命警告（以「警告：」前缀走 `on_error` 回调）。
    pub(crate) fn warning(&self, message: impl Into<String>) {
        (self.on_error)(format!("警告：{}", message.into()));
    }

    /// 上报错误。
    pub(crate) fn error(&self, message: impl Into<String>) {
        (self.on_error)(message.into());
    }

    /// 致命失败：明确停止整个引擎，`is_running` 立即变为 false。
    pub(crate) fn fatal(&self) {
        self.running.store(false, Ordering::SeqCst);
        self.stop.store(true, Ordering::SeqCst);
    }

    /// 发布一个定长音频块：噪声门 → RMS → 迟滞边 + 联合状态机 → 电平 → `on_chunk`。
    pub(crate) fn publish_chunk(&self, mut chunk: AudioChunk) {
        // 噪声门（若启用）：压低底噪，减少联合状态误判与 STT 噪声段。
        if let Some(threshold) = self.config.noise_gate_threshold {
            chunk.samples = apply_noise_gate(&chunk.samples, threshold);
        }
        let rms = crate::level::rms(&chunk.samples);
        // 迟滞边 + 联合状态机：只在真实状态迁移时回调 on_speech_edge。
        let now_ms = self.epoch.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let edge = {
            let mut detectors = lock(&self.detectors);
            let detector = match chunk.channel {
                Channel::Interviewer => &mut detectors[0],
                Channel::Me => &mut detectors[1],
            };
            detector.on_rms(rms)
        };
        let transition = edge.and_then(|speaking| {
            let mut joint = lock(&self.joint);
            joint.on_edge(joint_channel(chunk.channel), speaking, now_ms)
        });
        if let Some(transition) = transition {
            (self.on_speech_edge)(transition);
        }
        self.levels
            .update(chunk.channel, rms, self.on_levels.as_ref());
        (self.on_chunk)(chunk);
    }

    /// 通道退出时把该通道电平清零、重置其迟滞检测器，并让联合状态机标记静默。
    pub(crate) fn finish_channel(&self, channel: Channel) {
        self.levels
            .update_force(channel, 0.0, self.on_levels.as_ref());
        let now_ms = self.epoch.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        {
            let mut detectors = lock(&self.detectors);
            let detector = match channel {
                Channel::Interviewer => &mut detectors[0],
                Channel::Me => &mut detectors[1],
            };
            detector.reset();
        }
        let transition = {
            let mut joint = lock(&self.joint);
            joint.reset_channel(joint_channel(channel), now_ms)
        };
        if let Some(transition) = transition {
            (self.on_speech_edge)(transition);
        }
    }

    /// 可被停止请求打断的退避等待。
    pub(crate) fn sleep_backoff(&self, attempt: u32) {
        let total =
            Duration::from_millis(u64::from(attempt.max(1)) * 300).min(Duration::from_secs(2));
        let deadline = Instant::now() + total;
        while !self.should_stop() {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            std::thread::sleep((deadline - now).min(Duration::from_millis(50)));
        }
    }
}

/// hint-audio 的 `Channel` → 联合状态机的 `JointChannel`。
fn joint_channel(channel: Channel) -> JointChannel {
    match channel {
        Channel::Interviewer => JointChannel::Interviewer,
        Channel::Me => JointChannel::Me,
    }
}

/// 一个后台采集线程。
pub(crate) struct Worker {
    pub(crate) name: &'static str,
    pub(crate) id: ThreadId,
    pub(crate) handle: JoinHandle<()>,
}

/// 音频引擎句柄。
///
/// - [`AudioHandle::stop`] 幂等；停止后线程被回收；
/// - [`AudioHandle::is_running`] 反映真实状态：用户停止或任一通道
///   连续失败被判定为致命后返回 `false`；
/// - `Drop` 时自动调用 `stop`。
pub struct AudioHandle {
    runtime: Arc<Runtime>,
    workers: Mutex<Vec<Worker>>,
}

impl AudioHandle {
    pub(crate) fn new(runtime: Arc<Runtime>, workers: Vec<Worker>) -> Self {
        Self {
            runtime,
            workers: Mutex::new(workers),
        }
    }

    /// 停止采集并等待线程退出（幂等）。
    ///
    /// 若在 `on_chunk` / `on_levels` 回调内部调用，会跳过对当前线程的
    /// `join`，避免自等待；其余线程仍会被回收。
    pub fn stop(&self) {
        self.runtime.stop.store(true, Ordering::SeqCst);
        self.runtime.running.store(false, Ordering::SeqCst);

        let current = std::thread::current().id();
        let workers = {
            let mut guard = lock(&self.workers);
            std::mem::take(&mut *guard)
        };
        for worker in workers {
            if worker.id == current {
                tracing::debug!("忽略对当前线程 {} 的 join", worker.name);
                continue;
            }
            if worker.handle.join().is_err() {
                tracing::warn!("采集线程 {} 退出时发生 panic", worker.name);
            }
        }
    }

    /// 引擎是否仍在运行。
    pub fn is_running(&self) -> bool {
        self.runtime.running.load(Ordering::SeqCst)
    }
}

impl Drop for AudioHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_is_idempotent_and_updates_running_state() {
        let runtime = Arc::new(Runtime::new(
            AudioConfig::default(),
            |_chunk| {},
            |_levels| {},
            |_message| {},
            |_transition| {},
        ));
        let handle = AudioHandle::new(runtime, Vec::new());
        assert!(handle.is_running());
        handle.stop();
        assert!(!handle.is_running());
        // 第二次调用不应 panic，也不改变状态。
        handle.stop();
        assert!(!handle.is_running());
    }
}
