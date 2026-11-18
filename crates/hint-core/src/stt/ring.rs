//! 每路音频的环形缓冲：保存最近一段时间的 16k 单声道 PCM，供 STT 断线补录回放。
//!
//! 参照 natively-cluely 的 `PcmRingBuffer`。纯数据结构，便于单测。

use std::collections::VecDeque;

/// 一块音频（16k 单声道 f32）。
#[derive(Clone, Debug)]
pub struct PcmFrame {
    /// PCM 样本。
    pub samples: Vec<f32>,
    /// 本帧首样本在会话时间轴上的位置（毫秒）。
    pub start_ms: i64,
    /// 本帧时长（毫秒）。
    pub dur_ms: u64,
    /// 本帧 RMS（写入时算好，供切片段使用）。
    pub rms: f32,
}

/// 每路一个的环形缓冲。
pub struct PcmRingBuffer {
    frames: VecDeque<PcmFrame>,
    cap_ms: u64,
    total_ms: u64,
}

impl PcmRingBuffer {
    pub fn new(cap_ms: u64) -> Self {
        Self {
            frames: VecDeque::new(),
            cap_ms,
            total_ms: 0,
        }
    }

    /// 推入一帧；返回为守住容量上限而淘汰的最旧帧（按序）。
    pub fn push(&mut self, frame: PcmFrame) -> Vec<PcmFrame> {
        self.total_ms += frame.dur_ms;
        self.frames.push_back(frame);
        let mut evicted = Vec::new();
        while self.total_ms > self.cap_ms {
            let Some(front) = self.frames.pop_front() else {
                break;
            };
            self.total_ms = self.total_ms.saturating_sub(front.dur_ms);
            evicted.push(front);
        }
        evicted
    }

    /// 取出与 `[from_ms, to_ms]` 有重叠的帧（按序，克隆）。
    pub fn range(&self, from_ms: i64, to_ms: i64) -> Vec<PcmFrame> {
        if to_ms <= from_ms {
            return Vec::new();
        }
        self.frames
            .iter()
            .filter(|frame| {
                let end = frame.start_ms + frame.dur_ms as i64;
                frame.start_ms < to_ms && end > from_ms
            })
            .cloned()
            .collect()
    }

    /// 最旧帧的起始时间。
    pub fn oldest_start_ms(&self) -> Option<i64> {
        self.frames.front().map(|frame| frame.start_ms)
    }

    /// 当前缓存总时长（毫秒）。
    pub fn total_ms(&self) -> u64 {
        self.total_ms
    }

    pub fn clear(&mut self) {
        self.frames.clear();
        self.total_ms = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(start_ms: i64, dur_ms: u64, rms: f32) -> PcmFrame {
        PcmFrame {
            samples: vec![0.0; 10],
            start_ms,
            dur_ms,
            rms,
        }
    }

    #[test]
    fn evicts_oldest_when_over_capacity() {
        let mut ring = PcmRingBuffer::new(100);
        let ev1 = ring.push(frame(0, 60, 0.1));
        assert!(ev1.is_empty());
        let ev2 = ring.push(frame(60, 60, 0.1));
        // 总 120 > 100，淘汰第一帧。
        assert_eq!(ev2.len(), 1);
        assert_eq!(ev2[0].start_ms, 0);
        assert_eq!(ring.total_ms(), 60);
        assert_eq!(ring.oldest_start_ms(), Some(60));
    }

    #[test]
    fn range_returns_overlapping_frames() {
        let mut ring = PcmRingBuffer::new(1000);
        ring.push(frame(0, 100, 0.1));
        ring.push(frame(100, 100, 0.1));
        ring.push(frame(200, 100, 0.1));
        // [50, 250] 与三帧都重叠。
        assert_eq!(ring.range(50, 250).len(), 3);
        // [150, 250] 与第二、三帧重叠。
        assert_eq!(ring.range(150, 250).len(), 2);
        // [300, 400] 无交集。
        assert!(ring.range(300, 400).is_empty());
        // 空区间。
        assert!(ring.range(100, 100).is_empty());
    }

    #[test]
    fn clear_empties_buffer() {
        let mut ring = PcmRingBuffer::new(1000);
        ring.push(frame(0, 100, 0.1));
        ring.clear();
        assert_eq!(ring.total_ms(), 0);
        assert!(ring.oldest_start_ms().is_none());
        assert!(ring.range(0, 100).is_empty());
    }
}
