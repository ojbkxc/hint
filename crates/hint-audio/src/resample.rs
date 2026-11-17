//! rubato 5 流式重采样封装：任意输入采样率 → 目标采样率单声道 f32。
//!
//! 设计要点：
//! - 输入侧按约 20 ms 一帧喂给 `Async` 重采样器（`FixedAsync::Input`），
//!   调用方可以传入任意长度的块；
//! - 输出侧丢弃重采样器固有的启动延迟帧，使时间轴与输入对齐；
//! - 输入采样率等于目标采样率时完全旁路，零延迟、零 CPU 开销；
//! - 声道降混在采集层完成（麦克风回调 / WASAPI 字节转换），
//!   本模块的重采样器固定单声道输入输出。

use crate::error::{AudioError, Result};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};

/// 输入侧重采样分块时长（毫秒）。20 ms 兼顾延迟与 CPU。
const INPUT_CHUNK_MILLIS: f64 = 20.0;
/// sinc 滤波器长度；128 taps 对 16 kHz 语音足够，CPU 占用低。
const SINC_LEN: usize = 128;
/// sinc 过采样倍数。
const OVERSAMPLING: usize = 128;

/// 单声道流式重采样器。
pub(crate) struct MonoResampler {
    /// `None` 表示输入与目标采样率相同，直接旁路。
    resampler: Option<Async<f32>>,
    in_chunk: usize,
    in_buf: Vec<f32>,
    out_buf: Vec<f32>,
    /// 尚未凑够 `in_chunk` 的输入样本。
    pending: Vec<f32>,
    /// 还需要丢弃的输出帧数（抵消重采样器启动延迟）。
    delay_left: usize,
}

impl MonoResampler {
    /// 创建重采样器。`input_rate == target_rate` 时内部旁路。
    pub(crate) fn new(input_rate: u32, target_rate: u32) -> Result<Self> {
        if input_rate == 0 || target_rate == 0 {
            return Err(AudioError::InvalidConfig("采样率不能为 0".to_string()));
        }
        if input_rate == target_rate {
            return Ok(Self {
                resampler: None,
                in_chunk: 0,
                in_buf: Vec::new(),
                out_buf: Vec::new(),
                pending: Vec::new(),
                delay_left: 0,
            });
        }

        let in_chunk =
            ((f64::from(input_rate) * INPUT_CHUNK_MILLIS / 1000.0).round() as usize).max(64);
        let ratio = f64::from(target_rate) / f64::from(input_rate);
        let params = SincInterpolationParameters {
            sinc_len: SINC_LEN,
            f_cutoff: None,
            oversampling_factor: OVERSAMPLING,
            interpolation: SincInterpolationType::Linear,
            window: WindowFunction::BlackmanHarris2,
        };
        let resampler =
            Async::<f32>::new_sinc(ratio, 1.05, &params, in_chunk, 1, FixedAsync::Input)
                .map_err(|e| AudioError::Resample(format!("创建 sinc 重采样器失败：{e}")))?;
        let out_max = resampler.output_frames_max();
        if out_max == 0 {
            return Err(AudioError::Resample(
                "重采样器输出块长度为 0，请检查采样率配置".to_string(),
            ));
        }
        let delay_left = resampler.output_delay();
        Ok(Self {
            resampler: Some(resampler),
            in_chunk,
            in_buf: Vec::with_capacity(in_chunk),
            out_buf: vec![0.0; out_max],
            pending: Vec::with_capacity(in_chunk * 2),
            delay_left,
        })
    }

    /// 推入单声道输入样本，把重采样后的样本追加到 `out`。
    ///
    /// 输入可以是任意长度；内部按 20 ms 分块处理。
    pub(crate) fn push(&mut self, mono: &[f32], out: &mut Vec<f32>) -> Result<()> {
        let Some(resampler) = self.resampler.as_mut() else {
            out.extend_from_slice(mono);
            return Ok(());
        };

        self.pending.extend_from_slice(mono);
        while self.pending.len() >= self.in_chunk {
            self.in_buf.clear();
            self.in_buf.extend(self.pending.drain(..self.in_chunk));

            let in_adapter = InterleavedSlice::new(&self.in_buf, 1, self.in_chunk)
                .map_err(|e| AudioError::Resample(format!("创建输入适配器失败：{e}")))?;
            let out_frames = self.out_buf.len();
            let mut out_adapter = InterleavedSlice::new_mut(&mut self.out_buf, 1, out_frames)
                .map_err(|e| AudioError::Resample(format!("创建输出适配器失败：{e}")))?;
            let (_consumed, produced) = resampler
                .process_into_buffer(&in_adapter, &mut out_adapter, None)
                .map_err(|e| AudioError::Resample(format!("重采样处理失败：{e}")))?;

            let produced = produced.min(out_frames);
            let mut chunk = &self.out_buf[..produced];
            if self.delay_left > 0 {
                let skip = self.delay_left.min(chunk.len());
                self.delay_left -= skip;
                chunk = &chunk[skip..];
            }
            out.extend_from_slice(chunk);
        }
        Ok(())
    }
}

/// 交错多声道 f32 → 单声道 f32（按帧求平均），仅用于单元测试。
#[cfg(test)]
pub(crate) fn downmix_interleaved(interleaved: &[f32], channels: usize) -> Vec<f32> {
    let mut out = Vec::new();
    if channels == 0 || interleaved.is_empty() {
        return out;
    }
    if channels == 1 {
        out.extend_from_slice(interleaved);
        return out;
    }
    let frames = interleaved.len() / channels;
    out.reserve(frames);
    let scale = 1.0 / channels as f32;
    for frame in interleaved.chunks_exact(channels) {
        let mut sum = 0.0f32;
        for &sample in frame {
            sum += sample;
        }
        out.push(sum * scale);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, freq: f32, seconds: f32, amplitude: f32) -> Vec<f32> {
        let count = (rate as f32 * seconds) as usize;
        let step = std::f32::consts::TAU * freq / rate as f32;
        (0..count)
            .map(|i| amplitude * (step * i as f32).sin())
            .collect()
    }

    fn collect_resampled(mut resampler: MonoResampler, input: &[f32], block: usize) -> Vec<f32> {
        let mut out = Vec::new();
        for chunk in input.chunks(block) {
            resampler.push(chunk, &mut out).expect("重采样失败");
        }
        out
    }

    fn count_positive_crossings(samples: &[f32]) -> usize {
        samples
            .windows(2)
            .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
            .count()
    }

    #[test]
    fn downmix_stereo_averages_channels() {
        let input = [1.0f32, -1.0, 0.5, 0.5, 0.25, 0.75];
        let mono = downmix_interleaved(&input, 2);
        assert_eq!(mono.len(), 3);
        assert!((mono[0] - 0.0).abs() < 1e-6);
        assert!((mono[1] - 0.5).abs() < 1e-6);
        assert!((mono[2] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn downmix_mono_is_passthrough() {
        let input = [0.1f32, -0.2, 0.3];
        let mono = downmix_interleaved(&input, 1);
        assert_eq!(mono, input);
    }

    #[test]
    fn passthrough_when_rates_equal() {
        let input: Vec<f32> = (0..1000).map(|i| (i as f32) * 0.001).collect();
        let mut resampler = MonoResampler::new(16_000, 16_000).expect("构造失败");
        let mut out = Vec::new();
        resampler.push(&input, &mut out).expect("处理失败");
        assert_eq!(out, input);
    }

    #[test]
    fn resample_48k_to_16k_keeps_duration_and_amplitude() {
        let input = sine(48_000, 440.0, 1.0, 0.5);
        let resampler = MonoResampler::new(48_000, 16_000).expect("构造失败");
        let output = collect_resampled(resampler, &input, 1024);
        let expected = 16_000i64;
        let delta = (output.len() as i64 - expected).abs();
        assert!(delta < 300, "输出长度 {} 与期望偏差过大", output.len());
        let rms_value = crate::level::rms(&output[64..]);
        assert!(
            (0.30..0.40).contains(&rms_value),
            "重采样后 RMS 异常：{rms_value}"
        );
    }

    #[test]
    fn resample_44k1_to_16k_keeps_duration() {
        let input = sine(44_100, 440.0, 1.0, 0.5);
        let resampler = MonoResampler::new(44_100, 16_000).expect("构造失败");
        let output = collect_resampled(resampler, &input, 960);
        let delta = (output.len() as i64 - 16_000).abs();
        assert!(delta < 400, "输出长度 {} 与期望偏差过大", output.len());
    }

    #[test]
    fn resample_48k_to_16k_preserves_frequency() {
        let input = sine(48_000, 440.0, 1.0, 0.5);
        let resampler = MonoResampler::new(48_000, 16_000).expect("构造失败");
        let output = collect_resampled(resampler, &input, 777);
        let duration = output.len() as f32 / 16_000.0;
        let estimated = count_positive_crossings(&output) as f32 / duration;
        assert!(
            (estimated - 440.0).abs() < 25.0,
            "频率估计偏差过大：{estimated} Hz"
        );
    }

    #[test]
    fn upsampling_8k_to_16k_works() {
        let input = sine(8_000, 440.0, 0.5, 0.5);
        let resampler = MonoResampler::new(8_000, 16_000).expect("构造失败");
        let output = collect_resampled(resampler, &input, 160);
        let delta = (output.len() as i64 - 8_000).abs();
        assert!(delta < 300, "输出长度 {} 与期望偏差过大", output.len());
    }
}
