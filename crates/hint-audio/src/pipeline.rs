//! 单通道处理流水线：单声道输入 → 重采样 → 按 `chunk_samples` 切块。
//!
//! 两个采集线程（`hint-mic` / `hint-system-audio`）各持有一个
//! [`ChannelPipeline`]，把降混后的单声道样本交给它，得到定长
//! [`AudioChunk`]；下游 VAD 只关心定长 16 kHz 块。

use crate::error::{AudioError, Result};
use crate::resample::MonoResampler;
use crate::{AudioChunk, Channel};

/// 去直流高通的截止频率（Hz）。
///
/// 取 60 Hz：低于人声基频（成年男声约 85 Hz 起、女声约 165 Hz 起），
/// 所以不会削掉语音；但足以滤掉设备直流偏置与 50/60 Hz 工频干扰。
const DC_BLOCK_CUTOFF_HZ: f32 = 60.0;

/// 单通道流水线。
pub(crate) struct ChannelPipeline {
    channel: Channel,
    target_rate: u32,
    chunk_samples: usize,
    resampler: MonoResampler,
    /// 已重采样但尚未凑够一块的样本。
    pending: Vec<f32>,
    /// 一阶高通（去直流）系数，按**输入**采样率算。
    ///
    /// 真机实测过：某些设备的麦克风端点会持续输出直流偏置
    /// （静默时 `mean=+0.169`、99% 能量落在 1 Hz 以下），
    /// 这种偏置会让 rms 电平虚高、把噪声判成"有人在说"，
    /// 并作为低频污染送进识别。送去直流是采集侧的标准预处理，
    /// 与具体设备无关。
    dc_coeff: f32,
    /// 高通的上一个输入/输出样本（滤波器状态）。
    dc_prev_in: f32,
    dc_prev_out: f32,
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
        // 一阶高通 y[n] = x[n] - x[n-1] + R·y[n-1]，R = 1 - 2π·fc/fs。
        // 系数用输入采样率算：去直流发生在重采样之前。
        let dc_coeff = 1.0 - 2.0 * std::f32::consts::PI * DC_BLOCK_CUTOFF_HZ / input_rate as f32;
        Ok(Self {
            channel,
            target_rate,
            chunk_samples,
            resampler,
            pending: Vec::with_capacity(chunk_samples * 2),
            dc_coeff: dc_coeff.clamp(0.0, 1.0),
            dc_prev_in: 0.0,
            dc_prev_out: 0.0,
        })
    }

    /// 去掉直流偏置（原地处理）。
    ///
    /// 一阶高通：`y[n] = x[n] - x[n-1] + R·y[n-1]`。
    /// 状态跨调用保留，所以分块推送与一次性推送结果一致。
    fn remove_dc(&mut self, samples: &mut [f32]) {
        for sample in samples.iter_mut() {
            let input = *sample;
            let output = input - self.dc_prev_in + self.dc_coeff * self.dc_prev_out;
            self.dc_prev_in = input;
            self.dc_prev_out = output;
            *sample = output;
        }
    }

    /// 推入单声道（已降混）样本，返回本次凑满的所有定长块。
    pub(crate) fn push_mono(&mut self, mono: &[f32]) -> Result<Vec<AudioChunk>> {
        // 去直流必须在重采样之前：偏置会随重采样一起被平滑，之后再滤代价更高，
        // 而且滤波器的截止频率要按输入采样率算才对得上。
        let mut cleaned = mono.to_vec();
        self.remove_dc(&mut cleaned);

        let mut resampled = Vec::new();
        self.resampler.push(&cleaned, &mut resampled)?;
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
        assert_eq!(chunks[0].channel, Channel::Me);
        assert_eq!(chunks[0].sample_rate, 16_000);

        // 去直流会改变样本的绝对值（一阶高通），所以这里断言**顺序保持**这一不变量：
        // 输入是严格递增的斜坡，高通后相邻差值仍为正（信号方向不变）。
        // 绝对值的正确性由 remove_dc_* 系列测试负责。
        for chunk in &chunks {
            for pair in chunk.samples.windows(2) {
                assert!(
                    pair[1] > pair[0],
                    "斜坡信号经去直流后仍应单调递增：{} -> {}",
                    pair[0],
                    pair[1]
                );
            }
        }

        // 剩余 276 个样本，凑不满一块。
        let empty = pipeline.push_mono(&[]).expect("处理失败");
        assert!(empty.is_empty());

        let tail = vec![0.0f32; 236];
        let chunks = pipeline.push_mono(&tail).expect("处理失败");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].samples.len(), 512);
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
    fn remove_dc_eliminates_constant_bias() {
        // 真机实测：某些麦克风端点静默时持续输出直流偏置
        // （mean=+0.169、99% 能量在 1 Hz 以下），会让 rms 虚高、
        // 把噪声判成"有人在说"，并作为低频污染送进识别。
        let mut pipeline =
            ChannelPipeline::new(Channel::Me, 16_000, 16_000, 16_000).expect("创建失败");

        let bias = 0.25f32;
        let mut input = vec![bias; 16_000];
        // 在偏置上叠一点语音带信号，确认高通不会把它一起抹掉。
        for (i, sample) in input.iter_mut().enumerate() {
            *sample += 0.2 * (std::f32::consts::TAU * 440.0 * i as f32 / 16_000.0).sin();
        }

        let chunks = pipeline.push_mono(&input).expect("处理失败");
        assert_eq!(chunks.len(), 1);
        let out = &chunks[0].samples;

        // 跳过起始瞬态（一阶高通收敛约需几百个样本），看后半段均值。
        let settled = &out[2_000..];
        let mean: f32 = settled.iter().sum::<f32>() / settled.len() as f32;
        assert!(
            mean.abs() < 0.01,
            "直流偏置应被压到接近 0，实际 mean={mean}"
        );

        // 440 Hz 在通带内，幅度应基本保留（允许滤波器的轻微衰减）。
        let peak = settled.iter().fold(0.0f32, |acc, v| acc.max(v.abs()));
        assert!(peak > 0.15, "440 Hz 语音带信号被过度衰减：peak={peak}");
    }

    #[test]
    fn remove_dc_state_survives_chunked_pushes() {
        // 分块推送与一次性推送的去直流结果必须一致：
        // 滤波器状态要跨调用保留，否则每个块都会产生一次起始瞬态。
        let input: Vec<f32> = (0..4_096)
            .map(|i| 0.3 + 0.2 * (std::f32::consts::TAU * 300.0 * i as f32 / 16_000.0).sin())
            .collect();

        let mut whole = ChannelPipeline::new(Channel::Me, 16_000, 16_000, 4_096).expect("a");
        let whole_out = whole.push_mono(&input).expect("a").remove(0).samples;

        let mut split = ChannelPipeline::new(Channel::Me, 16_000, 16_000, 4_096).expect("b");
        let mut split_out = Vec::new();
        for block in input.chunks(256) {
            for chunk in split.push_mono(block).expect("b") {
                split_out.extend_from_slice(&chunk.samples);
            }
        }

        assert_eq!(whole_out.len(), split_out.len());
        let max_diff = whole_out
            .iter()
            .zip(split_out.iter())
            .fold(0.0f32, |acc, (a, b)| acc.max((a - b).abs()));
        assert!(
            max_diff < 1e-6,
            "分块推送与整段推送结果不一致，max_diff={max_diff}"
        );
    }

    #[test]
    fn zero_chunk_size_is_rejected() {
        let result = ChannelPipeline::new(Channel::Me, 16_000, 16_000, 0);
        assert!(result.is_err());
    }
}
