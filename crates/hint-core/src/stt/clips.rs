//! 把环形缓冲里的帧序列切成语音片段（供断线补录逐段重新识别）。

use crate::stt::ring::PcmFrame;

/// 片段切分参数。
#[derive(Clone, Copy, Debug)]
pub struct ClipOptions {
    /// 判定「有语音」的 RMS 阈值。
    pub speech_rms: f32,
    /// 语音不足此时长的片段视为噪声丢弃。
    pub min_speech_ms: u64,
    /// 静音超过此时长即断句。
    pub break_silence_ms: u64,
    /// 单片段最长时长（超过即强制切分）。
    pub max_clip_ms: u64,
    /// 片段尾部保留的静音（缓解丢字）。
    pub pad_ms: u64,
}

impl Default for ClipOptions {
    fn default() -> Self {
        Self {
            speech_rms: 0.012,
            min_speech_ms: 400,
            break_silence_ms: 800,
            max_clip_ms: 25_000,
            pad_ms: 300,
        }
    }
}

/// 一个语音片段。
#[derive(Clone, Debug)]
pub struct SpeechClip {
    pub samples: Vec<f32>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub speech_ms: u64,
}

/// 把帧序列切成语音片段。
///
/// 连续累积帧，静音超过 `break_silence_ms` 或时长超过 `max_clip_ms` 时切段；
/// 语音不足 `min_speech_ms` 的段被丢弃。`pad_ms` 由断句静音自然覆盖。
pub fn split_into_speech_clips(frames: &[PcmFrame], opts: &ClipOptions) -> Vec<SpeechClip> {
    let mut clips = Vec::new();
    let mut current: Vec<PcmFrame> = Vec::new();
    let mut speech_ms = 0u64;
    let mut silence_ms = 0u64;
    let mut current_ms = 0u64;

    for frame in frames {
        let loud = frame.rms >= opts.speech_rms;
        current.push(frame.clone());
        current_ms += frame.dur_ms;
        if loud {
            speech_ms += frame.dur_ms;
            silence_ms = 0;
        } else {
            silence_ms += frame.dur_ms;
        }
        let too_long = current_ms >= opts.max_clip_ms;
        let broken = silence_ms >= opts.break_silence_ms;
        if too_long || broken {
            flush(
                &mut clips,
                &mut current,
                &mut speech_ms,
                &mut silence_ms,
                &mut current_ms,
                opts,
            );
        }
    }
    flush(
        &mut clips,
        &mut current,
        &mut speech_ms,
        &mut silence_ms,
        &mut current_ms,
        opts,
    );
    clips
}

fn flush(
    clips: &mut Vec<SpeechClip>,
    current: &mut Vec<PcmFrame>,
    speech_ms: &mut u64,
    silence_ms: &mut u64,
    current_ms: &mut u64,
    opts: &ClipOptions,
) {
    if current.is_empty() {
        return;
    }
    if *speech_ms >= opts.min_speech_ms {
        let Some(first) = current.first() else {
            return;
        };
        let start_ms = first.start_ms;
        let end_ms = current
            .last()
            .map(|frame| frame.start_ms + frame.dur_ms as i64)
            .unwrap_or(start_ms);
        let samples: Vec<f32> = current
            .iter()
            .flat_map(|frame| frame.samples.iter().copied())
            .collect();
        clips.push(SpeechClip {
            samples,
            start_ms,
            end_ms,
            speech_ms: *speech_ms,
        });
    }
    current.clear();
    *speech_ms = 0;
    *silence_ms = 0;
    *current_ms = 0;
    let _ = opts.pad_ms;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(start_ms: i64, dur_ms: u64, rms: f32) -> PcmFrame {
        PcmFrame {
            samples: vec![0.0; dur_ms as usize * 16],
            start_ms,
            dur_ms,
            rms,
        }
    }

    #[test]
    fn silence_only_produces_no_clip() {
        let frames = vec![frame(0, 100, 0.0), frame(100, 100, 0.0)];
        let clips = split_into_speech_clips(&frames, &ClipOptions::default());
        assert!(clips.is_empty());
    }

    #[test]
    fn speech_above_min_produces_clip() {
        let frames = vec![frame(0, 500, 0.5)];
        let clips = split_into_speech_clips(&frames, &ClipOptions::default());
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].speech_ms, 500);
    }

    #[test]
    fn short_speech_below_min_is_dropped() {
        let frames = vec![frame(0, 200, 0.5)];
        let clips = split_into_speech_clips(&frames, &ClipOptions::default());
        assert!(clips.is_empty());
    }

    #[test]
    fn long_silence_breaks_clips() {
        // 语音 500 + 静音 900 + 语音 500 → 两段。
        let mut frames = vec![frame(0, 500, 0.5)];
        frames.push(frame(500, 900, 0.0));
        frames.push(frame(1400, 500, 0.5));
        let clips = split_into_speech_clips(&frames, &ClipOptions::default());
        assert_eq!(clips.len(), 2);
    }

    #[test]
    fn max_clip_truncates() {
        let opts = ClipOptions {
            max_clip_ms: 1000,
            ..ClipOptions::default()
        };
        let frames: Vec<_> = (0..30).map(|i| frame(i * 100, 100, 0.5)).collect();
        let clips = split_into_speech_clips(&frames, &opts);
        assert!(clips.len() >= 2);
    }

    #[test]
    fn empty_input_produces_no_clip() {
        assert!(split_into_speech_clips(&[], &ClipOptions::default()).is_empty());
    }
}
