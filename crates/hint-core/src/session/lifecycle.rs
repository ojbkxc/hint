//! 会话生命周期队列：串行化 start/stop 请求，消除 check-exec 竞态。
//!
//! 规则（参照 natively-cluely 的 `meetingLifecycleQueue`）：
//! - **同向合并**：在途或排队尾部的同向请求不重复入队（last intent wins）；
//! - **异向排队**：在途请求完成前，异向请求入队等待，完成后自动接续；
//! - **已达成**：队列为空且当前状态已是目标时返回 `Noop`；
//! - **失败回写**：start 失败回 Idle，stop 失败回 Active（会话其实还在跑）。
//!
//! 纯逻辑、无线程、无 I/O，便于单测。执行编排（锁内决策、锁外执行）在
//! `src-tauri/src/session.rs`。

use std::collections::VecDeque;

/// 会话生命周期请求方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleDir {
    Start,
    Stop,
}

/// 生命周期状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LifecycleState {
    #[default]
    Idle,
    Starting,
    Active,
    Stopping,
}

/// 一次 `request` 的决策结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleAction {
    /// 需要立即执行该方向（调用方收到后调 [`LifecycleQueue::begin`] 再执行）。
    Run(LifecycleDir),
    /// 已并入同向的在途/排队请求，无需重复执行。
    Coalesced,
    /// 已处于目标状态且队列空，无需执行。
    Noop,
    /// 已排队，等待在途的异向请求完成。
    Queued,
}

/// 生命周期队列。
#[derive(Default)]
pub struct LifecycleQueue {
    state: LifecycleState,
    /// 当前在途请求的方向；无在途时为 `None`。
    in_flight: Option<LifecycleDir>,
    /// 等待执行的排队请求。
    pending: VecDeque<LifecycleDir>,
}

impl LifecycleQueue {
    /// 当前状态。
    pub fn state(&self) -> LifecycleState {
        self.state
    }

    /// 会话是否在运行。
    pub fn is_active(&self) -> bool {
        self.state == LifecycleState::Active
    }

    /// 是否有请求在途（正在执行，尚未结束）。
    pub fn in_flight(&self) -> bool {
        self.in_flight.is_some()
    }

    /// 目标状态是否已达成。
    fn at_target(&self, dir: LifecycleDir) -> bool {
        match dir {
            LifecycleDir::Start => self.state == LifecycleState::Active,
            LifecycleDir::Stop => self.state == LifecycleState::Idle,
        }
    }

    /// 请求一个生命周期动作，返回决策结果。
    ///
    /// 同向重复请求被合并；目标已达成且队列空时为 `Noop`；否则入队，
    /// 无在途请求时返回 `Run`（调用方负责调 [`begin`] 并执行）。
    pub fn request(&mut self, dir: LifecycleDir) -> LifecycleAction {
        // 同向合并：在途或排队尾部同向，不重复入队。
        if self.in_flight == Some(dir) || self.pending.back() == Some(&dir) {
            return LifecycleAction::Coalesced;
        }
        // 已达成：队列空且当前状态已是目标。
        if self.pending.is_empty() && self.at_target(dir) {
            return LifecycleAction::Noop;
        }
        self.pending.push_back(dir);
        if self.in_flight.is_none() {
            let front = self
                .pending
                .pop_front()
                .expect("刚入队，队首必然存在");
            return LifecycleAction::Run(front);
        }
        LifecycleAction::Queued
    }

    /// 开始执行一个方向：置在途标志并切换到 Starting / Stopping。
    pub fn begin(&mut self, dir: LifecycleDir) {
        self.in_flight = Some(dir);
        self.state = match dir {
            LifecycleDir::Start => LifecycleState::Starting,
            LifecycleDir::Stop => LifecycleState::Stopping,
        };
    }

    /// 在途请求结束，回写状态。
    ///
    /// start 成功 → Active，失败 → Idle；stop 成功 → Idle，失败 → Active
    /// （停止失败意味着会话其实还在跑，不能误报为空闲）。
    pub fn finish(&mut self, dir: LifecycleDir, ok: bool) {
        self.in_flight = None;
        self.state = match (dir, ok) {
            (LifecycleDir::Start, true) => LifecycleState::Active,
            (LifecycleDir::Start, false) => LifecycleState::Idle,
            (LifecycleDir::Stop, true) => LifecycleState::Idle,
            (LifecycleDir::Stop, false) => LifecycleState::Active,
        };
    }

    /// 取下一个待执行的请求，跳过已达成目标的项。
    ///
    /// 返回 `None` 表示队列已空（或全部已达成）。
    pub fn next_pending(&mut self) -> Option<LifecycleDir> {
        while let Some(front) = self.pending.front() {
            if self.at_target(*front) {
                self.pending.pop_front();
                continue;
            }
            return self.pending.pop_front();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_start_runs_and_active_start_is_noop() {
        let mut q = LifecycleQueue::default();
        assert_eq!(q.state(), LifecycleState::Idle);
        assert_eq!(
            q.request(LifecycleDir::Start),
            LifecycleAction::Run(LifecycleDir::Start)
        );
        q.begin(LifecycleDir::Start);
        q.finish(LifecycleDir::Start, true);
        assert_eq!(q.state(), LifecycleState::Active);
        // 已在 Active，再 Start → Noop。
        assert_eq!(q.request(LifecycleDir::Start), LifecycleAction::Noop);
    }

    #[test]
    fn idle_stop_is_noop_and_active_stop_runs() {
        let mut q = LifecycleQueue::default();
        assert_eq!(q.request(LifecycleDir::Stop), LifecycleAction::Noop);
        q.begin(LifecycleDir::Start);
        q.finish(LifecycleDir::Start, true);
        assert_eq!(
            q.request(LifecycleDir::Stop),
            LifecycleAction::Run(LifecycleDir::Stop)
        );
    }

    #[test]
    fn same_direction_is_coalesced() {
        let mut q = LifecycleQueue::default();
        assert_eq!(
            q.request(LifecycleDir::Start),
            LifecycleAction::Run(LifecycleDir::Start)
        );
        q.begin(LifecycleDir::Start);
        // 在途时再 Start → 合并，不重复入队。
        assert_eq!(q.request(LifecycleDir::Start), LifecycleAction::Coalesced);
        // 停止时再 Stop → 合并。
        q.finish(LifecycleDir::Start, true);
        assert_eq!(
            q.request(LifecycleDir::Stop),
            LifecycleAction::Run(LifecycleDir::Stop)
        );
        q.begin(LifecycleDir::Stop);
        assert_eq!(q.request(LifecycleDir::Stop), LifecycleAction::Coalesced);
    }

    #[test]
    fn queued_tail_same_direction_is_coalesced() {
        let mut q = LifecycleQueue::default();
        q.begin(LifecycleDir::Start);
        // 启动中来 Stop → 排队。
        assert_eq!(q.request(LifecycleDir::Stop), LifecycleAction::Queued);
        // 排队尾部仍是 Stop，再来 Stop → 合并。
        assert_eq!(q.request(LifecycleDir::Stop), LifecycleAction::Coalesced);
    }

    #[test]
    fn opposite_request_queues_and_drains_after_finish() {
        let mut q = LifecycleQueue::default();
        assert_eq!(
            q.request(LifecycleDir::Start),
            LifecycleAction::Run(LifecycleDir::Start)
        );
        q.begin(LifecycleDir::Start);
        // 启动中来 Stop → 排队。
        assert_eq!(q.request(LifecycleDir::Stop), LifecycleAction::Queued);
        // 启动成功 → Active，drain 取出 Stop。
        q.finish(LifecycleDir::Start, true);
        assert_eq!(q.state(), LifecycleState::Active);
        assert_eq!(q.next_pending(), Some(LifecycleDir::Stop));
    }

    #[test]
    fn start_failure_returns_to_idle_and_skips_queued_stop() {
        let mut q = LifecycleQueue::default();
        q.begin(LifecycleDir::Start);
        assert_eq!(q.request(LifecycleDir::Stop), LifecycleAction::Queued);
        // 启动失败 → Idle；排队中的 Stop 已达成（目标 Idle），被跳过。
        q.finish(LifecycleDir::Start, false);
        assert_eq!(q.state(), LifecycleState::Idle);
        assert_eq!(q.next_pending(), None);
    }

    #[test]
    fn stop_failure_returns_to_active_and_skips_queued_start() {
        let mut q = LifecycleQueue::default();
        q.begin(LifecycleDir::Start);
        q.finish(LifecycleDir::Start, true);
        q.begin(LifecycleDir::Stop);
        assert_eq!(q.request(LifecycleDir::Start), LifecycleAction::Queued);
        // 停止失败 → Active；排队中的 Start 已达成（目标 Active），被跳过。
        q.finish(LifecycleDir::Stop, false);
        assert_eq!(q.state(), LifecycleState::Active);
        assert_eq!(q.next_pending(), None);
    }

    #[test]
    fn start_stop_start_converges_to_active_with_single_execution() {
        let mut q = LifecycleQueue::default();
        // Start → Run，开始执行。
        assert_eq!(
            q.request(LifecycleDir::Start),
            LifecycleAction::Run(LifecycleDir::Start)
        );
        q.begin(LifecycleDir::Start);
        // Start → 合并。
        assert_eq!(q.request(LifecycleDir::Start), LifecycleAction::Coalesced);
        // Stop → 排队。
        assert_eq!(q.request(LifecycleDir::Stop), LifecycleAction::Queued);
        // 启动成功 → Active，drain 执行 Stop。
        q.finish(LifecycleDir::Start, true);
        let stop = q.next_pending().expect("应取出 Stop");
        q.begin(stop);
        // Stop → 合并。
        assert_eq!(q.request(LifecycleDir::Stop), LifecycleAction::Coalesced);
        // Start → 排队。
        assert_eq!(q.request(LifecycleDir::Start), LifecycleAction::Queued);
        // 停止成功 → Idle，drain 执行 Start。
        q.finish(stop, true);
        let start = q.next_pending().expect("应取出 Start");
        q.begin(start);
        q.finish(start, true);
        assert_eq!(q.state(), LifecycleState::Active);
        // 全程只执行了一次 Start、一次 Stop（合并的都被吸收）。
    }

    #[test]
    fn concurrent_toggle_sequence_converges() {
        // 模拟多次 toggle：Start,Start,Stop,Stop 全并发到达。
        let mut q = LifecycleQueue::default();
        // 第一个 Start → Run。
        assert_eq!(
            q.request(LifecycleDir::Start),
            LifecycleAction::Run(LifecycleDir::Start)
        );
        q.begin(LifecycleDir::Start);
        // 第二个 Start → 合并。
        assert_eq!(q.request(LifecycleDir::Start), LifecycleAction::Coalesced);
        // 第一个 Stop → 排队。
        assert_eq!(q.request(LifecycleDir::Stop), LifecycleAction::Queued);
        // 第二个 Stop → 合并（排队尾部同向）。
        assert_eq!(q.request(LifecycleDir::Stop), LifecycleAction::Coalesced);
        // 启动成功 → drain 执行 Stop，只执行一次。
        q.finish(LifecycleDir::Start, true);
        let stop = q.next_pending().expect("应取出 Stop");
        q.begin(stop);
        q.finish(stop, true);
        assert_eq!(q.state(), LifecycleState::Idle);
    }
}
