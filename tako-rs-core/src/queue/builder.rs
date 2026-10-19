use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;

use parking_lot::Mutex;
use scc::HashMap as SccHashMap;
use tokio::sync::Notify;

use super::RetryPolicy;
use super::runtime::Queue;
use super::runtime::QueueInner;

/// Configures a [`Queue`], from [`Queue::builder`].
pub struct QueueBuilder {
  pub(crate) workers: usize,
  pub(crate) retry: RetryPolicy,
}

impl QueueBuilder {
  /// Number of worker tasks, at least 1 (default 4).
  pub fn workers(mut self, n: usize) -> Self {
    self.workers = n.max(1);
    self
  }

  /// Retry policy for failed jobs (default: none).
  pub fn retry(mut self, policy: RetryPolicy) -> Self {
    self.retry = policy;
    self
  }

  /// Builds the queue; it runs jobs only after [`Queue::start`].
  pub fn build(self) -> Queue {
    Queue {
      inner: Arc::new(QueueInner {
        pending: Mutex::default(),
        handlers: SccHashMap::new(),
        dead_letters: Mutex::new(Vec::new()),
        notify: Notify::new(),
        next_id: AtomicU64::new(1),
        num_workers: self.workers,
        retry_policy: self.retry,
        shutdown: AtomicBool::new(false),
        inflight: AtomicU64::new(0),
        drain_notify: Notify::new(),
      }),
    }
  }
}
