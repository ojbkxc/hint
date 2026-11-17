//! RMS 电平计算与 `on_levels` 回调节流。
//!
//! 两个采集线程各自把每块音频的 RMS 写入共享的 [`LevelsBus`]，
//! 并由总线按约 30 fps（33 ms）的间隔统一回调一次 [`Levels`]，
//! 避免双通道各自回调导致前端收到两倍频率的更新。

use crate::{Channel, Levels};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

/// 电平回调的最小间隔（毫秒）。
const LEVELS_INTERVAL_MS: u64 = 33;

/// 计算一段 f32 采样的均方根（RMS）。
///
/// 空切片返回 0；NaN / 无穷采样会被忽略，避免脏数据污染 UI 电平条。
pub(crate) fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sum = 0.0f64;
    let mut count = 0u64;
    for &sample in samples {
        if sample.is_finite() {
            sum += f64::from(sample) * f64::from(sample);
            count += 1;
        }
    }
    if count == 0 {
        return 0.0;
    }
    (sum / count as f64).sqrt() as f32
}

/// 双通道电平总线。
///
/// 两个采集线程都写入同一个总线；任意线程在距上次回调超过 33 ms 时
/// 触发一次 `on_levels`（CAS 保证全局只触发一次），因此整体约 30 fps。
pub(crate) struct LevelsBus {
    mic_rms: AtomicU32,
    system_rms: AtomicU32,
    /// 上次回调时刻（相对 `epoch` 的毫秒数）；0 表示尚未回调过。
    last_emit_ms: AtomicU64,
    epoch: Instant,
}

impl LevelsBus {
    pub(crate) fn new() -> Self {
        Self {
            mic_rms: AtomicU32::new(0),
            system_rms: AtomicU32::new(0),
            last_emit_ms: AtomicU64::new(0),
            epoch: Instant::now(),
        }
    }

    /// 更新某个通道的 RMS，并按 30 fps 节流回调。
    pub(crate) fn update(&self, channel: Channel, value: f32, on_levels: &dyn Fn(Levels)) {
        self.store(channel, value);
        let now_ms = self.epoch.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let prev = self.last_emit_ms.load(Ordering::Relaxed);
        let due = prev == 0 || now_ms.saturating_sub(prev) >= LEVELS_INTERVAL_MS;
        if due
            && self
                .last_emit_ms
                .compare_exchange(prev, now_ms, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
        {
            on_levels(self.snapshot());
        }
    }

    /// 更新并强制回调一次（用于通道停止时把电平清零）。
    pub(crate) fn update_force(&self, channel: Channel, value: f32, on_levels: &dyn Fn(Levels)) {
        self.store(channel, value);
        let now_ms = self.epoch.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        self.last_emit_ms.store(now_ms, Ordering::Relaxed);
        on_levels(self.snapshot());
    }

    /// 读取当前两通道电平快照。
    pub(crate) fn snapshot(&self) -> Levels {
        Levels {
            mic_rms: f32::from_bits(self.mic_rms.load(Ordering::Relaxed)),
            system_rms: f32::from_bits(self.system_rms.load(Ordering::Relaxed)),
        }
    }

    fn store(&self, channel: Channel, value: f32) {
        let sanitized = if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let bits = sanitized.to_bits();
        match channel {
            Channel::Me => self.mic_rms.store(bits, Ordering::Relaxed),
            Channel::Interviewer => self.system_rms.store(bits, Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn rms_of_empty_is_zero() {
        assert_eq!(rms(&[]), 0.0);
    }

    #[test]
    fn rms_of_silence_is_zero() {
        assert_eq!(rms(&[0.0; 512]), 0.0);
    }

    #[test]
    fn rms_of_dc_equals_amplitude() {
        let value = rms(&[0.5; 512]);
        assert!((value - 0.5).abs() < 1e-4, "rms = {value}");
        let negative = rms(&[-0.5; 512]);
        assert!((negative - 0.5).abs() < 1e-4, "rms = {negative}");
    }

    #[test]
    fn rms_of_full_scale_square_is_one() {
        let mut samples = [0.0f32; 512];
        for (i, sample) in samples.iter_mut().enumerate() {
            *sample = if i % 2 == 0 { 1.0 } else { -1.0 };
        }
        assert!((rms(&samples) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn rms_ignores_non_finite_values() {
        let samples = [f32::NAN, f32::INFINITY, 0.5, 0.5];
        assert!((rms(&samples) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn levels_bus_force_emits_snapshot() {
        let bus = LevelsBus::new();
        let captured: Arc<Mutex<Vec<Levels>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = captured.clone();
        bus.update_force(Channel::Interviewer, 0.25, &|levels| {
            if let Ok(mut guard) = sink.lock() {
                guard.push(levels);
            }
        });
        let values = captured.lock().expect("锁可用");
        assert_eq!(values.len(), 1);
        assert!((values[0].system_rms - 0.25).abs() < 1e-4);
        assert_eq!(values[0].mic_rms, 0.0);
        assert!((bus.snapshot().system_rms - 0.25).abs() < 1e-4);
    }
}
