//! 采集回调与处理线程之间的单声道样本队列。
//!
//! cpal 的数据回调运行在音频线程上，不能做重采样等重活；回调只把
//! 降混后的单声道 f32 样本推入本队列，由通道处理线程消费。
//! 队列有容量上限，溢出时丢弃最旧的数据，绝不阻塞音频回调。

use crate::lock;
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

struct QueueInner {
    data: VecDeque<f32>,
    closed: bool,
}

/// 有界单声道样本队列（多生产者单消费者）。
pub(crate) struct SampleQueue {
    inner: Mutex<QueueInner>,
    cond: Condvar,
    capacity: usize,
}

impl SampleQueue {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(QueueInner {
                data: VecDeque::new(),
                closed: false,
            }),
            cond: Condvar::new(),
            capacity: capacity.max(1),
        }
    }

    /// 推入样本；队列已关闭时静默丢弃。溢出时丢弃最旧的样本。
    pub(crate) fn push(&self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        let mut guard = lock(&self.inner);
        if guard.closed {
            return;
        }
        guard.data.extend(samples.iter().copied());
        if guard.data.len() > self.capacity {
            let overflow = guard.data.len() - self.capacity;
            guard.data.drain(..overflow);
        }
        drop(guard);
        self.cond.notify_one();
    }

    /// 等待最多 `timeout`，取走当前所有可用样本并写入 `out`。
    ///
    /// 返回 `false` 表示队列已关闭且已排空，调用方应结束处理循环。
    pub(crate) fn take(&self, timeout: Duration, out: &mut Vec<f32>) -> bool {
        let mut guard = lock(&self.inner);
        if guard.data.is_empty() && !guard.closed {
            let (next, _) = self
                .cond
                .wait_timeout(guard, timeout)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard = next;
        }
        out.clear();
        out.extend(guard.data.drain(..));
        !guard.closed
    }

    /// 关闭队列并唤醒等待方；之后推入的数据会被丢弃。
    pub(crate) fn close(&self) {
        let mut guard = lock(&self.inner);
        guard.closed = true;
        guard.data.clear();
        drop(guard);
        self.cond.notify_all();
    }
}
