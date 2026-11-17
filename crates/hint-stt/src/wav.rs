//! WAV 解析：只接受桌面录音链路产出的「44 字节标准头 + 16k 单声道 PCM16」。
//!
//! 上层（前端）已做格式归一，这里只守两件事：头部标记正确、data 长度不越界。
//! 解析失败一律返回 `InvalidInput`（worker 回 400）。

use crate::error::{Result, SttError};
use crate::types::{MAX_AUDIO_SAMPLES, MAX_AUDIO_SECONDS};

/// 标准 WAV 头长度。
pub const HEADER_BYTES: usize = 44;

/// 解析出的 PCM。
#[derive(Debug, Clone)]
pub struct PcmAudio {
    /// 归一化到 [-1, 1) 的采样。
    pub samples: Vec<f32>,
    /// 采样率，恒为 16000。
    pub sample_rate: u32,
}

impl PcmAudio {
    /// 录音秒数。
    pub fn seconds(&self) -> f32 {
        self.samples.len() as f32 / self.sample_rate as f32
    }
}

/// 读取 16k 单声道 PCM16 WAV。
pub fn read_pcm16_mono_16k(bytes: &[u8]) -> Result<PcmAudio> {
    if bytes.len() < HEADER_BYTES {
        return Err(invalid("audio is shorter than a 44-byte WAV header"));
    }
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" || &bytes[36..40] != b"data" {
        return Err(invalid("audio is not a canonical 44-byte-header WAV file"));
    }
    let channels = u16::from_le_bytes([bytes[22], bytes[23]]);
    let sample_rate = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
    let bits = u16::from_le_bytes([bytes[34], bytes[35]]);
    if channels != 1 {
        return Err(invalid("audio must be mono"));
    }
    if sample_rate != 16_000 {
        return Err(invalid("audio must be 16 kHz"));
    }
    if bits != 16 {
        return Err(invalid("audio must be PCM16"));
    }
    let data_bytes = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]) as usize;
    if data_bytes == 0 || data_bytes % 2 != 0 {
        return Err(invalid("WAV data length is empty or odd"));
    }
    if data_bytes > bytes.len() - HEADER_BYTES {
        return Err(invalid("WAV data length runs past the end of the file"));
    }
    let sample_count = data_bytes / 2;
    if sample_count > MAX_AUDIO_SAMPLES {
        return Err(invalid("audio is longer than the 120 second limit"));
    }
    let mut samples = Vec::with_capacity(sample_count);
    for chunk in bytes[HEADER_BYTES..HEADER_BYTES + data_bytes].chunks_exact(2) {
        let value = i16::from_le_bytes([chunk[0], chunk[1]]);
        samples.push(value as f32 / 32_768.0);
    }
    if samples.len() as f32 / 16_000.0 > MAX_AUDIO_SECONDS {
        return Err(invalid("audio is longer than the 120 second limit"));
    }
    Ok(PcmAudio {
        samples,
        sample_rate: 16_000,
    })
}

fn invalid(message: impl Into<String>) -> SttError {
    SttError::InvalidInput {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 生成一个标准 44 字节头的 16k 单声道 PCM16 WAV。
    pub(crate) fn make_wav(samples: &[i16]) -> Vec<u8> {
        let data_bytes = (samples.len() * 2) as u32;
        let mut out = Vec::with_capacity(HEADER_BYTES + data_bytes as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // mono
        out.extend_from_slice(&16_000u32.to_le_bytes());
        out.extend_from_slice(&32_000u32.to_le_bytes()); // byte rate
        out.extend_from_slice(&2u16.to_le_bytes()); // block align
        out.extend_from_slice(&16u16.to_le_bytes()); // bits
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_bytes.to_le_bytes());
        for sample in samples {
            out.extend_from_slice(&sample.to_le_bytes());
        }
        out
    }

    #[test]
    fn parses_a_canonical_wav() {
        let wav = make_wav(&[0, 32767, -32768, 1000]);
        let pcm = read_pcm16_mono_16k(&wav).unwrap();
        assert_eq!(pcm.sample_rate, 16_000);
        assert_eq!(pcm.samples.len(), 4);
        assert!((pcm.samples[0]).abs() < f32::EPSILON);
        assert!((pcm.samples[1] - 32767.0 / 32768.0).abs() < f32::EPSILON);
        assert!((pcm.samples[2] + 1.0).abs() < f32::EPSILON);
        assert!((pcm.seconds() - 4.0 / 16_000.0).abs() < 1e-6);
    }

    #[test]
    fn rejects_wrong_markers_and_shapes() {
        let wav = make_wav(&[1, 2, 3, 4]);
        let mut bad = wav.clone();
        bad[0] = b'X';
        assert!(read_pcm16_mono_16k(&bad).is_err());

        let mut stereo = wav.clone();
        stereo[22] = 2;
        assert!(read_pcm16_mono_16k(&stereo).is_err());

        let mut rate = wav.clone();
        rate[24..28].copy_from_slice(&44_100u32.to_le_bytes());
        assert!(read_pcm16_mono_16k(&rate).is_err());

        let mut bits = wav.clone();
        bits[34..36].copy_from_slice(&24u16.to_le_bytes());
        assert!(read_pcm16_mono_16k(&bits).is_err());

        let mut odd = wav.clone();
        odd[40..44].copy_from_slice(&3u32.to_le_bytes());
        assert!(read_pcm16_mono_16k(&odd).is_err());

        let mut overrun = wav.clone();
        overrun[40..44].copy_from_slice(&10_000u32.to_le_bytes());
        assert!(read_pcm16_mono_16k(&overrun).is_err());

        assert!(read_pcm16_mono_16k(b"short").is_err());
    }

    #[test]
    fn rejects_audio_over_the_limit() {
        let samples = vec![0i16; MAX_AUDIO_SAMPLES + 2];
        let wav = make_wav(&samples);
        let error = read_pcm16_mono_16k(&wav).unwrap_err();
        assert!(matches!(error, SttError::InvalidInput { .. }));
    }
}
