//! STT 断线跟踪：stall 看门狗、outage 开闭、指数退避。
//!
//! 参照 natively-cluely 的 `SttGapTracker`，适配 hint 的离线 SenseVoice worker。
//! 纯逻辑、时钟注入（`now_ms` 参数），便于单测。

use crate::audio::JointChannel;
use crate::stt::ring::{PcmFrame, PcmRingBuffer};

/// 断线原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutageReason {
    /// 识别报错。
    Error,
    /// 有语音输入但长时间无识别结果。
    Stall,
}

/// 一个断线窗口。
#[derive(Clone, Debug)]
pub struct Outage {
    pub id: u64,
    pub channel: JointChannel,
    pub reason: OutageReason,
    /// 断线窗口起点（error 时已回溯 preroll）。
    pub start_ms: i64,
    /// 恢复时刻；补录用窗口终点。
    pub recovered_at_ms: Option<i64>,
    /// 窗口内因环形容量被淘汰、无法补录的时长。
    pub dropped_ms: u64,
}

/// 单通道的断线跟踪器。
pub struct SttGapTracker {
    channel: JointChannel,
    ring: PcmRingBuffer,
    outage: Option<Outage>,
    /// 上次识别结果（或启动）的时刻。
    quiet_since_ms: Option<i64>,
    /// quiet 以来累计的语音时长。
    speech_ms_since_quiet: u64,
    consecutive_stalls: u32,
    live_finals: Vec<(i64, String)>,
    drop_warned: bool,
    outage_seq: u64,
    /// 是否已经产出过至少一次识别结果。用于区分「冷启动期」与「热路径」：
    /// worker 冷启最长 60s，首个结果到达前的看门狗门槛应放宽，否则会把
    /// 正常冷启误判为 stall 并反复回收引擎。
    has_transcript: bool,
}

impl SttGapTracker {
    pub fn new(channel: JointChannel, cap_ms: u64) -> Self {
        Self {
            channel,
            ring: PcmRingBuffer::new(cap_ms),
            outage: None,
            quiet_since_ms: None,
            speech_ms_since_quiet: 0,
            consecutive_stalls: 0,
            live_finals: Vec::new(),
            drop_warned: false,
            outage_seq: 0,
            has_transcript: false,
        }
    }

    /// 采集路径：推入环形缓冲 + 累计语音 + 看门狗判定。
    ///
    /// 返回 true 表示本次判定为 stall（调用方应强制回收 worker）。
    #[allow(clippy::too_many_arguments)]
    pub fn on_audio(
        &mut self,
        samples: &[f32],
        start_ms: i64,
        dur_ms: u64,
        rms: f32,
        speech_rms: f32,
        stall_ms: u64,
        stall_speech_ms: u64,
        max_stall_backoff_ms: u64,
    ) -> bool {
        let now_ms = start_ms + dur_ms as i64;
        let frame = PcmFrame {
            samples: samples.to_vec(),
            start_ms,
            dur_ms,
            rms,
        };
        let evicted = self.ring.push(frame);
        if let Some(outage) = self.outage.as_mut() {
            // 只统计落在本窗口内的音频：早于 outage 起点的旧帧本来就不在补录
            // 范围内（参考实现同样按 `end > outage.start` 过滤），算进去会虚报。
            let outage_start = outage.start_ms;
            let dropped: u64 = evicted
                .iter()
                .filter(|frame| frame.start_ms + frame.dur_ms as i64 > outage_start)
                .map(|frame| frame.dur_ms)
                .sum();
            if dropped > 0 {
                outage.dropped_ms += dropped;
                if !self.drop_warned {
                    self.drop_warned = true;
                    tracing::warn!(dropped, "断线窗口内音频超过环形容量，部分无法补录");
                }
            }
        }
        if self.quiet_since_ms.is_none() {
            self.quiet_since_ms = Some(now_ms);
        }
        if rms >= speech_rms {
            self.speech_ms_since_quiet += dur_ms;
        }
        let backoff = stall_backoff_ms(self.consecutive_stalls, stall_ms, max_stall_backoff_ms);
        let quiet_for = now_ms - self.quiet_since_ms.unwrap_or(now_ms);
        if quiet_for >= backoff as i64 && self.speech_ms_since_quiet >= stall_speech_ms {
            if self.outage.is_none() {
                self.outage_seq += 1;
                self.outage = Some(Outage {
                    id: self.outage_seq,
                    channel: self.channel,
                    reason: OutageReason::Stall,
                    start_ms: self.quiet_since_ms.unwrap_or(now_ms),
                    recovered_at_ms: None,
                    dropped_ms: 0,
                });
            }
            self.consecutive_stalls += 1;
            self.quiet_since_ms = Some(now_ms);
            self.speech_ms_since_quiet = 0;
            return true;
        }
        false
    }

    /// 识别出错：开 outage，起点回溯 `error_preroll_ms`。
    pub fn on_error(&mut self, now_ms: i64, error_preroll_ms: u64) {
        if self.outage.is_none() {
            self.outage_seq += 1;
            self.outage = Some(Outage {
                id: self.outage_seq,
                channel: self.channel,
                reason: OutageReason::Error,
                start_ms: now_ms - error_preroll_ms as i64,
                recovered_at_ms: None,
                dropped_ms: 0,
            });
        }
    }

    /// 识别成功：关闭 outage（若有），记录 live final。返回被关闭的 outage。
    pub fn on_transcript(&mut self, text: &str, now_ms: i64) -> Option<Outage> {
        self.quiet_since_ms = Some(now_ms);
        self.speech_ms_since_quiet = 0;
        self.consecutive_stalls = 0;
        self.drop_warned = false;
        // 收到结果即说明 worker 已就绪：此后看门狗恢复常规门槛。
        self.has_transcript = true;
        // 空结果（纯噪声段）说明 worker 活着、可以关闭 outage，但不值得记入
        // 去重用的 live 文本池。
        if !text.trim().is_empty() {
            self.live_finals.push((now_ms, text.to_string()));
        }
        // 只保留最近 120 秒的 live final，避免无限增长。
        self.live_finals.retain(|(ms, _)| now_ms - ms <= 120_000);
        match self.outage.take() {
            Some(mut outage) => {
                outage.recovered_at_ms = Some(now_ms);
                Some(outage)
            }
            None => None,
        }
    }

    /// 窗口 `[from, to]` 内的实时识别文本（补录去重用）。
    pub fn live_texts_between(&self, from_ms: i64, to_ms: i64) -> Vec<String> {
        self.live_finals
            .iter()
            .filter(|(ms, _)| *ms >= from_ms && *ms <= to_ms)
            .map(|(_, text)| text.clone())
            .collect()
    }

    pub fn ring(&self) -> &PcmRingBuffer {
        &self.ring
    }

    /// 是否已产出过识别结果（区分冷启动期与热路径的看门狗门槛）。
    pub fn has_transcript(&self) -> bool {
        self.has_transcript
    }

    pub fn reset(&mut self) {
        self.ring.clear();
        self.outage = None;
        self.quiet_since_ms = None;
        self.speech_ms_since_quiet = 0;
        self.consecutive_stalls = 0;
        self.live_finals.clear();
        self.drop_warned = false;
        self.has_transcript = false;
    }
}

/// 连续 stall 的指数退避（对齐参考实现，上限 `max_ms`）。
pub fn stall_backoff_ms(consecutive: u32, stall_ms: u64, max_ms: u64) -> u64 {
    let shift = consecutive.min(8);
    let scaled = stall_ms.saturating_mul(1u64 << shift);
    scaled.min(max_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker() -> SttGapTracker {
        SttGapTracker::new(JointChannel::Interviewer, 120_000)
    }

    #[test]
    fn stall_backoff_increases_and_caps() {
        assert_eq!(stall_backoff_ms(0, 15_000, 120_000), 15_000);
        assert_eq!(stall_backoff_ms(1, 15_000, 120_000), 30_000);
        assert_eq!(stall_backoff_ms(2, 15_000, 120_000), 60_000);
        assert_eq!(stall_backoff_ms(10, 15_000, 120_000), 120_000);
    }

    #[test]
    fn stall_requires_speech_input() {
        let mut t = tracker();
        // 全静音：即使超过 stall_ms 也不触发。
        assert!(!t.on_audio(&[0.0], 0, 20_000, 0.0, 0.012, 15_000, 4_000, 120_000));
    }

    #[test]
    fn stall_fires_after_speech_and_timeout() {
        let mut t = tracker();
        // 5s 语音，quiet_since=5000，未超时。
        assert!(!t.on_audio(&[0.5], 0, 5_000, 0.5, 0.012, 15_000, 4_000, 120_000));
        // 静音到 now=16000，quiet_for=11000 < 15000。
        assert!(!t.on_audio(&[0.0], 5_000, 11_000, 0.0, 0.012, 15_000, 4_000, 120_000));
        // 静音到 now=21000，quiet_for=16000 >= 15000 → stall。
        assert!(t.on_audio(&[0.0], 16_000, 5_000, 0.0, 0.012, 15_000, 4_000, 120_000));
    }

    #[test]
    fn error_opens_outage_and_transcript_closes() {
        let mut t = tracker();
        t.on_error(10_000, 3_000);
        let outage = t.on_transcript("识别到文本", 12_000);
        let outage = outage.expect("应关闭 outage");
        assert_eq!(outage.start_ms, 7_000);
        assert_eq!(outage.recovered_at_ms, Some(12_000));
    }

    #[test]
    fn live_texts_between_filters_by_time() {
        let mut t = tracker();
        t.on_transcript("早", 1_000);
        t.on_transcript("晚", 10_000);
        let texts = t.live_texts_between(5_000, 15_000);
        assert_eq!(texts, vec!["晚".to_string()]);
    }

    #[test]
    fn reset_clears_state() {
        let mut t = tracker();
        t.on_error(1_000, 500);
        t.reset();
        assert!(t.ring().oldest_start_ms().is_none());
        // reset 后 outage 已清，on_transcript 不再返回 outage。
        assert!(t.on_transcript("x", 2_000).is_none());
    }

    #[test]
    fn has_transcript_tracks_first_result_and_resets() {
        let mut t = tracker();
        assert!(!t.has_transcript());
        // 空结果也说明 worker 活着，应置位。
        t.on_transcript("", 1_000);
        assert!(t.has_transcript());
        t.reset();
        assert!(!t.has_transcript());
    }

    /// 断线窗口内音频超过环形容量时，`dropped_ms` 要如实累计。
    ///
    /// 这是「用户说的话丢了」的**量化证据**：环形缓冲只保留最近
    /// `cap_ms`（默认 120 秒），断线期间若说话超过这个时长，超出部分无法
    /// 补录。`dropped_ms` 就是被丢掉的时长——它进日志和诊断，若统计错了，
    /// 排查时会以为音频没丢。
    ///
    /// 此前这条逻辑**没有任何测试**：`outage.dropped_ms += dropped` 写在哪
    /// 都不影响现有断言，改错（比如漏掉 `> outage_start` 过滤）也不会红。
    #[test]
    fn dropped_ms_counts_only_audio_inside_outage_window() {
        let mut t = SttGapTracker::new(JointChannel::Interviewer, 5_000);
        // 开断线窗口，起点 10_000（error 回溯 3_000）。
        t.on_error(13_000, 3_000);

        // 窗口内推入 4 秒音频，容量只有 5 秒 → 会淘汰。
        // 帧：start=13_000 dur=2_000、start=15_000 dur=2_000、
        //     start=17_000 dur=2_000、start=19_000 dur=2_000
        // 累计 8 秒，容量 5 秒，淘汰 3 秒。
        for (start, dur) in [
            (13_000, 2_000),
            (15_000, 2_000),
            (17_000, 2_000),
            (19_000, 2_000),
        ] {
            let _ = t.on_audio(&[0.5], start, dur, 0.5, 0.012, 15_000, 4_000, 120_000);
        }

        // 被关闭的 outage 里应累计被淘汰的音频时长。
        //
        // 淘汰过程（容量 5000ms，窗口内推 4 帧各 2000ms）：
        //   f1(13000) → total 2000
        //   f2(15000) → total 4000
        //   f3(17000) → total 6000 > 5000 → 淘汰 f1，total 4000
        //   f4(19000) → total 6000 > 5000 → 淘汰 f2，total 4000
        // 淘汰 f1、f2 各 2000ms，合计 4000ms。两帧都落在窗口内
        // （start + dur > outage.start = 10000）。
        let outage = t.on_transcript("恢复", 21_000).expect("应关闭 outage");
        assert_eq!(
            outage.dropped_ms, 4_000,
            "窗口内被淘汰的音频时长应累计为 4000ms，实际 {}",
            outage.dropped_ms
        );
    }

    /// 断线窗口**之外**被淘汰的音频**不应**计入 `dropped_ms`。
    ///
    /// 过滤条件 `start + dur > outage.start` 是必要的：进入 outage 之前的
    /// 旧帧本来就不在补录范围，算进去会让 `dropped_ms` 虚高，排查时误以为
    /// 窗口内丢了更多音频。
    #[test]
    fn dropped_ms_ignores_evictions_before_outage() {
        let mut t = SttGapTracker::new(JointChannel::Interviewer, 5_000);
        // 先在窗口外堆满环形缓冲（淘汰发生但不应计入）。
        for start in (0..3_000).step_by(1_000) {
            let _ = t.on_audio(&[0.5], start, 1_000, 0.5, 0.012, 15_000, 4_000, 120_000);
        }
        // 此时已淘汰 3 秒，但还没开窗口。
        // 开窗口，起点 10_000（回溯 3_000）。
        t.on_error(13_000, 3_000);

        // 窗口内只推 1 秒（容量 5 秒，已满，会淘汰，但淘汰的是窗口外的旧帧）
        let _ = t.on_audio(&[0.5], 13_000, 1_000, 0.5, 0.012, 15_000, 4_000, 120_000);

        let outage = t.on_transcript("恢复", 15_000).expect("应关闭 outage");
        // 被淘汰的帧 start 都在 0~3000，`start + dur` 最大 4000 < 10000，
        // 不落在窗口内 → dropped_ms 应为 0。
        assert_eq!(
            outage.dropped_ms, 0,
            "窗口外的淘汰不应计入 dropped_ms，实际 {}",
            outage.dropped_ms
        );
    }
}
