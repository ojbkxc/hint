//! 单通道处理流水线：单声道输入 → 重采样 → 按 `chunk_samples` 切块。
//!
//! 两个采集线程（`hint-mic` / `hint-system-audio`）各持有一个
//! [`ChannelPipeline`]，把降混后的单声道样本交给它，得到定长
//! [`AudioChunk`]；下游 VAD 只关心定长 16 kHz 块。

use crate::error::{AudioError, Result};
use crate::resample::MonoResampler;
use crate::{AudioChunk, Channel};

/// 单通道流水线。
pub(crate) struct ChannelPipeline {
    channel: Channel,
    target_rate: u32,
    chunk_samples: usize,
    resampler: MonoResampler,
    /// 已重采样但尚未凑够一块的样本。
    pending: Vec<f32>,
}

impl ChannelPipeline {
    /// 创建流水线。
    ///
    /// `input_rate` 是设备原生采样率；输出恒为 `target_rate` 单声道，
    /// 每 `chunk_samples` 个样本产出一个 [`AudioChunk`]。
    pub(crate) fn new(
        channel: Channel,
        input_rate: u32,
        target_rate: u32,
        chunk_samples: usize,
    ) -> Result<Self> {
        if chunk_samples == 0 {
            return Err(AudioError::InvalidConfig(
                "chunk_samples 不能为 0".to_string(),
            ));
        }
        let resampler = MonoResampler::new(input_rate, target_rate)?;
        Ok(Self {
            channel,
            target_rate,
            chunk_samples,
            resampler,
            pending: Vec::with_capacity(chunk_samples * 2),
        })
    }

    /// 推入单声道（已降混）样本，返回本次凑满的所有定长块。
    pub(crate) fn push_mono(&mut self, mono: &[f32]) -> Result<Vec<AudioChunk>> {
        let mut resampled = Vec::new();
        self.resampler.push(mono, &mut resampled)?;
        self.pending.extend_from_slice(&resampled);

        let mut chunks = Vec::new();
        while self.pending.len() >= self.chunk_samples {
            let rest = self.pending.split_off(self.chunk_samples);
            let samples = std::mem::replace(&mut self.pending, rest);
            chunks.push(AudioChunk {
                channel: self.channel,
                samples,
                sample_rate: self.target_rate,
            });
        }
        Ok(chunks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunker_emits_exact_block_sizes_and_preserves_order() {
        let mut pipeline =
            ChannelPipeline::new(Channel::Me, 16_000, 16_000, 512).expect("创建流水线失败");

        let input: Vec<f32> = (0..1300).map(|i| i as f32).collect();
        let chunks = pipeline.push_mono(&input).expect("处理失败");
        assert_eq!(chunks.len(), 2);
        assert!(chunks.iter().all(|chunk| chunk.samples.len() == 512));
        assert_eq!(chunks[0].samples[0], 0.0);
        assert_eq!(chunks[0].samples[511], 511.0);
        assert_eq!(chunks[1].samples[0], 512.0);
        assert_eq!(chunks[0].channel, Channel::Me);
        assert_eq!(chunks[0].sample_rate, 16_000);

        // 剩余 276 个样本，凑不满一块。
        let empty = pipeline.push_mono(&[]).expect("处理失败");
        assert!(empty.is_empty());

        let tail = vec![0.0f32; 236];
        let chunks = pipeline.push_mono(&tail).expect("处理失败");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].samples.len(), 512);
        assert_eq!(chunks[0].samples[0], 1024.0);
        assert_eq!(chunks[0].samples[275], 1299.0);
        assert_eq!(chunks[0].samples[276], 0.0);
    }

    #[test]
    fn pipeline_48k_input_emits_512_sample_16k_chunks() {
        let mut pipeline =
            ChannelPipeline::new(Channel::Interviewer, 48_000, 16_000, 512).expect("创建失败");
        let input: Vec<f32> = (0..48_000)
            .map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin() * 0.5)
            .collect();
        let mut total = 0usize;
        let mut chunks = 0usize;
        for block in input.chunks(1024) {
            for chunk in pipeline.push_mono(block).expect("处理失败") {
                assert_eq!(chunk.samples.len(), 512);
                assert_eq!(chunk.sample_rate, 16_000);
                total += chunk.samples.len();
                chunks += 1;
            }
        }
        assert!(chunks > 20, "块数量过少：{chunks}");
        assert!(
            (total as i64 - 16_000).abs() < 600,
            "总样本数 {total} 与期望偏差过大"
        );
    }

    #[test]
    fn zero_chunk_size_is_rejected() {
        let result = ChannelPipeline::new(Channel::Me, 16_000, 16_000, 0);
        assert!(result.is_err());
    }
}
