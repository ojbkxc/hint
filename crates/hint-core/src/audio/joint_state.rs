//! 双通道联合语音状态机：判断当前是面试官在说、我在说、还是两人都在说。
//!
//! 参照 natively-cluely 的 `JointSpeechState`（neither / interviewer_speaking /
//! user_speaking / both）+ `msSinceOtherEdge`。纯状态机、时钟注入（`now_ms`
//! 参数），无线程、无 I/O，便于单测。hint-audio 的 `Channel` 映射到
//! [`JointChannel`] 后喂入。

/// 联合语音状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointSpeechState {
    /// 两路都静默。
    Neither,
    /// 仅系统声（面试官）在说。
    InterviewerSpeaking,
    /// 仅麦克风（我）在说。
    MeSpeaking,
    /// 两路同时在说。
    Both,
}

/// 联合状态机的输入通道（hint-audio 的 `Channel` 映射到此处）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JointChannel {
    Interviewer,
    Me,
}

/// 一次状态迁移（只在真实变化时产生）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JointSpeechTransition {
    /// 触发本次迁移的通道。
    pub channel: JointChannel,
    /// 该通道本次边：`true`=开始说话，`false`=停止说话。
    pub speaking: bool,
    /// 迁移后的联合状态。
    pub joint: JointSpeechState,
    /// 事件发生时刻（引擎时间轴毫秒）。
    pub at_ms: u64,
    /// 距另一通道上次边的毫秒；另一通道从未有边时为 `u64::MAX`。
    pub ms_since_other_edge: u64,
}

/// 联合状态跟踪器。
pub struct JointSpeechTracker {
    interviewer_speaking: bool,
    me_speaking: bool,
    last_interviewer_edge_ms: Option<u64>,
    last_me_edge_ms: Option<u64>,
}

impl Default for JointSpeechTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl JointSpeechTracker {
    pub fn new() -> Self {
        Self {
            interviewer_speaking: false,
            me_speaking: false,
            last_interviewer_edge_ms: None,
            last_me_edge_ms: None,
        }
    }

    /// 当前联合状态。
    pub fn joint(&self) -> JointSpeechState {
        match (self.interviewer_speaking, self.me_speaking) {
            (false, false) => JointSpeechState::Neither,
            (true, false) => JointSpeechState::InterviewerSpeaking,
            (false, true) => JointSpeechState::MeSpeaking,
            (true, true) => JointSpeechState::Both,
        }
    }

    /// 喂入一条通道边；同向重复边被吸收，返回 `None`。
    pub fn on_edge(
        &mut self,
        channel: JointChannel,
        speaking: bool,
        now_ms: u64,
    ) -> Option<JointSpeechTransition> {
        // 同向重复边吸收：状态未变就不产生迁移。
        let is_same = match channel {
            JointChannel::Interviewer => self.interviewer_speaking == speaking,
            JointChannel::Me => self.me_speaking == speaking,
        };
        if is_same {
            return None;
        }
        // 更新该通道的说话状态与上次边时间。
        match channel {
            JointChannel::Interviewer => {
                self.interviewer_speaking = speaking;
                self.last_interviewer_edge_ms = Some(now_ms);
            }
            JointChannel::Me => {
                self.me_speaking = speaking;
                self.last_me_edge_ms = Some(now_ms);
            }
        }
        // 距另一通道上次边的毫秒数。
        let other_edge_ms = match channel {
            JointChannel::Interviewer => self.last_me_edge_ms,
            JointChannel::Me => self.last_interviewer_edge_ms,
        };
        let ms_since_other_edge = match other_edge_ms {
            Some(ms) if now_ms >= ms => now_ms - ms,
            _ => u64::MAX,
        };
        Some(JointSpeechTransition {
            channel,
            speaking,
            joint: self.joint(),
            at_ms: now_ms,
            ms_since_other_edge,
        })
    }

    /// 某通道采集停止时调用：只置该通道为静默，不影响另一通道。
    ///
    /// 该通道本就静默时返回 `None`（无变化）。
    pub fn reset_channel(
        &mut self,
        channel: JointChannel,
        now_ms: u64,
    ) -> Option<JointSpeechTransition> {
        // 直接走 on_edge(false)：内部会判断该通道是否已静默（无变化时返回 None）。
        self.on_edge(channel, false, now_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_is_neither() {
        let tracker = JointSpeechTracker::new();
        assert_eq!(tracker.joint(), JointSpeechState::Neither);
    }

    #[test]
    fn four_state_flow() {
        let mut tracker = JointSpeechTracker::new();
        let t = tracker
            .on_edge(JointChannel::Interviewer, true, 100)
            .unwrap();
        assert_eq!(t.joint, JointSpeechState::InterviewerSpeaking);
        let t = tracker.on_edge(JointChannel::Me, true, 200).unwrap();
        assert_eq!(t.joint, JointSpeechState::Both);
        let t = tracker
            .on_edge(JointChannel::Interviewer, false, 300)
            .unwrap();
        assert_eq!(t.joint, JointSpeechState::MeSpeaking);
        let t = tracker.on_edge(JointChannel::Me, false, 400).unwrap();
        assert_eq!(t.joint, JointSpeechState::Neither);
    }

    #[test]
    fn same_direction_edge_is_absorbed() {
        let mut tracker = JointSpeechTracker::new();
        tracker.on_edge(JointChannel::Interviewer, true, 100);
        // 再次同向 → None。
        assert!(
            tracker
                .on_edge(JointChannel::Interviewer, true, 150)
                .is_none()
        );
    }

    #[test]
    fn first_edge_ms_since_other_is_max() {
        let mut tracker = JointSpeechTracker::new();
        let t = tracker
            .on_edge(JointChannel::Interviewer, true, 100)
            .unwrap();
        assert_eq!(t.ms_since_other_edge, u64::MAX);
    }

    #[test]
    fn alternating_edges_compute_diff() {
        let mut tracker = JointSpeechTracker::new();
        tracker.on_edge(JointChannel::Interviewer, true, 100);
        let t = tracker.on_edge(JointChannel::Me, true, 250).unwrap();
        // 距面试官上次边 150ms。
        assert_eq!(t.ms_since_other_edge, 150);
        // 我停止：距面试官上次边（100）已过去 200ms。
        let t = tracker.on_edge(JointChannel::Me, false, 300).unwrap();
        assert_eq!(t.ms_since_other_edge, 200);
    }

    #[test]
    fn reset_channel_silences_only_that_channel() {
        let mut tracker = JointSpeechTracker::new();
        tracker.on_edge(JointChannel::Interviewer, true, 100);
        tracker.on_edge(JointChannel::Me, true, 200);
        // reset 面试官 → 只剩我。
        let t = tracker
            .reset_channel(JointChannel::Interviewer, 300)
            .unwrap();
        assert_eq!(t.joint, JointSpeechState::MeSpeaking);
        // reset 一个已经静默的通道 → None。
        assert!(
            tracker
                .reset_channel(JointChannel::Interviewer, 350)
                .is_none()
        );
    }

    #[test]
    fn reset_channel_is_idempotent_when_already_silent() {
        let mut tracker = JointSpeechTracker::new();
        assert!(tracker.reset_channel(JointChannel::Me, 100).is_none());
        assert_eq!(tracker.joint(), JointSpeechState::Neither);
    }
}
