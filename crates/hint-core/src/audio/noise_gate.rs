//! 音频指标与噪声门：RMS + peak、软膝噪声门、带迟滞的语音边缘检测。
//!
//! 参照 pluely 的 `calculate_audio_metrics` / `apply_noise_gate`。纯函数与
//! 纯状态，便于单测。

/// 计算 RMS 与 peak（忽略 NaN/Inf）。空切片或全为 NaN/Inf 时返回 (0, 0)。
pub fn audio_metrics(samples: &[f32]) -> (f32, f32) {
    if samples.is_empty() {
        return (0.0, 0.0);
    }
    let mut sum = 0.0f64;
    let mut peak = 0.0f32;
    let mut count = 0u64;
    for &sample in samples {
        if !sample.is_finite() {
            continue;
        }
        sum += f64::from(sample) * f64::from(sample);
        let abs = sample.abs();
        if abs > peak {
            peak = abs;
        }
        count += 1;
    }
    if count == 0 {
        return (0.0, 0.0);
    }
    let rms = (sum / count as f64).sqrt() as f32;
    (rms, peak)
}

/// 软膝噪声门：低于阈值的样本按 `abs/threshold` 比例衰减，abs 越小压得越狠；
/// 高于阈值的样本原样通过。
///
/// 这样底噪被压向零，语音主体（RMS ≥ 阈值的块）几乎不受影响，减少联合状态
/// 机误判与 STT 噪声段。`threshold <= 0` 或非有限时原样返回。
pub fn apply_noise_gate(samples: &[f32], threshold: f32) -> Vec<f32> {
    if !samples.iter().any(|s| s.is_finite()) || threshold <= 0.0 || !threshold.is_finite() {
        return samples.to_vec();
    }
    samples
        .iter()
        .map(|&sample| {
            if !sample.is_finite() {
                return sample;
            }
            let abs = sample.abs();
            if abs < threshold {
                sample * (abs / threshold)
            } else {
                sample
            }
        })
        .collect()
}

/// 带迟滞的语音边缘检测器：防止 RMS 在阈值附近抖动导致状态闪烁。
pub struct SpeechEdgeDetector {
    on_threshold: f32,
    off_threshold: f32,
    speaking: bool,
}

impl SpeechEdgeDetector {
    pub fn new(on_threshold: f32) -> Self {
        Self {
            on_threshold,
            off_threshold: on_threshold * 0.7,
            speaking: false,
        }
    }

    /// 输入一块的 RMS，返回本次是否产生边（`true`=开始，`false`=结束，`None`=无变化）。
    ///
    /// 低于 `on_threshold` 不触发开始；已开始后，只有低于 `off_threshold`
    /// （`on_threshold` 的 70%）才触发结束，中间抖动不产生边。
    pub fn on_rms(&mut self, rms: f32) -> Option<bool> {
        if !rms.is_finite() {
            return None;
        }
        if !self.speaking {
            if rms >= self.on_threshold {
                self.speaking = true;
                Some(true)
            } else {
                None
            }
        } else if rms < self.off_threshold {
            self.speaking = false;
            Some(false)
        } else {
            None
        }
    }

    pub fn reset(&mut self) {
        self.speaking = false;
    }
}

/// 语音判定 RMS 阈值（session.rs 的 `Segmenter` 与联合状态机共用，避免语义漂移）。
pub const SPEECH_RMS: f32 = 0.012;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_of_silence_and_dc() {
        assert_eq!(audio_metrics(&[]), (0.0, 0.0));
        assert_eq!(audio_metrics(&[0.0, 0.0]), (0.0, 0.0));
        let (rms, peak) = audio_metrics(&[0.5, 0.5]);
        assert_eq!(rms, 0.5);
        assert_eq!(peak, 0.5);
    }

    #[test]
    fn metrics_of_full_scale() {
        let (rms, peak) = audio_metrics(&[1.0, -1.0]);
        assert_eq!(rms, 1.0);
        assert_eq!(peak, 1.0);
    }

    #[test]
    fn metrics_ignore_nan() {
        let (rms, peak) = audio_metrics(&[0.0, f32::NAN, 1.0]);
        // 只有 0.0 与 1.0 参与：rms = sqrt((0+1)/2)。
        assert_eq!(rms, ((0.0 + 1.0) / 2.0).sqrt());
        assert_eq!(peak, 1.0);
    }

    #[test]
    fn noise_gate_compresses_below_threshold() {
        let threshold = 0.1;
        let out = apply_noise_gate(&[0.05, 0.0], threshold);
        // 0.05 < 0.1：衰减为 0.05 * (0.05/0.1) = 0.025，小于原值。
        assert!(out[0] < 0.05);
        assert_eq!(out[0], 0.025);
        assert_eq!(out[1], 0.0);
    }

    #[test]
    fn noise_gate_passes_above_threshold() {
        let threshold = 0.1;
        let out = apply_noise_gate(&[0.5, -0.5], threshold);
        assert_eq!(out[0], 0.5);
        assert_eq!(out[1], -0.5);
    }

    #[test]
    fn noise_gate_zero_threshold_passes_through() {
        let out = apply_noise_gate(&[0.5, 0.0], 0.0);
        assert_eq!(out, vec![0.5, 0.0]);
    }

    #[test]
    fn noise_gate_empty() {
        assert!(apply_noise_gate(&[], 0.1).is_empty());
    }

    #[test]
    fn edge_detector_hysteresis() {
        let mut det = SpeechEdgeDetector::new(0.1);
        // 低于 on → 无。
        assert!(det.on_rms(0.05).is_none());
        // 达到 on → true。
        assert_eq!(det.on_rms(0.1), Some(true));
        // 在 on/off 之间抖动 → 无（off = 0.07，0.08 > 0.07）。
        assert!(det.on_rms(0.08).is_none());
        // 低于 off → false。
        assert_eq!(det.on_rms(0.05), Some(false));
    }

    #[test]
    fn edge_detector_reset() {
        let mut det = SpeechEdgeDetector::new(0.1);
        assert_eq!(det.on_rms(0.2), Some(true));
        det.reset();
        // reset 后重新从 on 判定。
        assert_eq!(det.on_rms(0.15), Some(true));
    }

    #[test]
    fn gated_sequence_edges_are_true_then_false() {
        // 底噪（0.01）→ 语音（0.5）→ 静音（0.01）。
        let threshold = 0.1;
        let segments = [vec![0.01f32; 4], vec![0.5f32; 4], vec![0.01f32; 4]];
        let mut det = SpeechEdgeDetector::new(SPEECH_RMS);
        let mut edges = Vec::new();
        for seg in segments {
            let gated = apply_noise_gate(&seg, threshold);
            let (rms, _) = audio_metrics(&gated);
            if let Some(edge) = det.on_rms(rms) {
                edges.push(edge);
            }
        }
        assert_eq!(edges, vec![true, false]);
    }
}
