use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;

use parking_lot::Mutex;
use tokio::sync::Notify;

use super::QueueLimits;
use super::RetryPolicy;
use super::capacity::Budget;
use super::lifecycle::Lifecycle;
use super::lifecycle::QueueOwner;
use super::retention::Retention;
use super::runtime::Queue;
use super::runtime::QueueInner;

/// Configures a [`Queue`], from [`Queue::builder`].
pub struct QueueBuilder {
  pub(crate) workers: usize,
  pub(crate) retry: RetryPolicy,
  pub(crate) limits: QueueLimits,
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

  /// Limits outstanding payloads and retained dead letters.
  pub fn limits(mut self, limits: QueueLimits) -> Self {
    self.limits = limits;
    self
  }

  /// Builds the queue; it runs jobs only after [`Queue::start`].
  pub fn build(self) -> Queue {
    let inner = Arc::new(QueueInner {
      pending: Mutex::default(),
      budget: Arc::new(Budget::new(self.limits)),
      handlers: Mutex::default(),
      dead_letters: Mutex::new(Retention::new(self.limits)),
      notify: Notify::new(),
      next_id: AtomicU64::new(1),
      num_workers: self.workers,
      retry_policy: self.retry,
      shutdown: AtomicBool::new(false),
      inflight: AtomicU64::new(0),
      lifecycle: Lifecycle::default(),
    });
    Queue {
      _owner: Arc::new(QueueOwner(Arc::downgrade(&inner))),
      inner,
    }
  }
}
