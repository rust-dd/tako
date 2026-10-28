use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::Notify;

use super::DeadJob;
use super::Job;
use super::QueueBuilder;
use super::QueueError;
use super::QueueHandle;
use super::QueueLimits;
use super::RetryPolicy;
use super::capacity::Budget;
use super::lifecycle::Lifecycle;
use super::lifecycle::QueueOwner;
use super::pending::PendingQueue;
use super::retention::Retention;

pub(crate) type BoxHandler =
  Arc<dyn Fn(Job) -> Pin<Box<dyn Future<Output = Result<(), QueueError>> + Send>> + Send + Sync>;

pub(crate) struct QueueInner {
  pub(crate) pending: Mutex<PendingQueue>,
  pub(crate) budget: Arc<Budget>,
  pub(crate) handlers: Mutex<HashMap<String, BoxHandler>>,
  /// `Arc` entries let [`Queue::dead_letters`] snapshot without copying payloads.
  pub(crate) dead_letters: Mutex<Retention<Arc<DeadJob>>>,
  pub(crate) notify: Notify,
  pub(crate) next_id: AtomicU64,
  pub(crate) num_workers: usize,
  pub(crate) retry_policy: RetryPolicy,
  pub(crate) shutdown: AtomicBool,
  pub(crate) inflight: AtomicU64,
  pub(crate) lifecycle: Lifecycle,
}

/// In-memory background job queue; clones share it, and jobs run only after
/// [`start`](Queue::start).
#[derive(Clone)]
pub struct Queue {
  pub(crate) inner: Arc<QueueInner>,
  pub(crate) _owner: Arc<QueueOwner>,
}

impl Queue {
  /// A queue with 4 workers and no retries.
  pub fn new() -> Self {
    Self::builder().build()
  }

  pub fn builder() -> QueueBuilder {
    QueueBuilder {
      workers: 4,
      retry: RetryPolicy::default(),
      limits: QueueLimits::default(),
    }
  }

  /// Registers the handler for jobs named `name`, replacing any previous one.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// queue.register("process_order", |job: Job| async move {
  ///     let order_id = job.deserialize::<u64>()?;
  ///     println!("Processing order {order_id}");
  ///     Ok(())
  /// });
  /// ```
  pub fn register<F, Fut>(&self, name: impl Into<String>, handler: F)
  where
    F: Fn(Job) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), QueueError>> + Send + 'static,
  {
    let name = name.into();
    let handler: BoxHandler = Arc::new(move |job| Box::pin(handler(job)));
    let previous = {
      let mut handlers = self.inner.handlers.lock();
      if self.inner.shutdown.load(Ordering::SeqCst) {
        return;
      }
      handlers.insert(name, handler)
    };
    drop(previous);
  }

  /// Starts the configured workers once, including across clones and concurrent calls.
  ///
  /// Dropping the last queue clone stops workers unless a [`QueueHandle`] remains.
  pub fn start(&self) {
    self.inner.start_workers();
  }

  /// Starts workers once and returns a shared [`QueueHandle`].
  pub fn spawn(&self) -> QueueHandle {
    let handle = self.inner.handle();
    self.inner.start_workers();
    handle
  }

  /// Stops the queue with a deadline; use [`QueueHandle::shutdown`] to receive errors.
  pub async fn shutdown(&self, timeout: Duration) {
    let _ = self.inner.shutdown_workers(timeout).await;
  }

  /// Snapshot of the dead letter queue that shares each entry.
  pub fn dead_letters(&self) -> Vec<Arc<DeadJob>> {
    self.inner.dead_letters.lock().iter().cloned().collect()
  }

  pub fn dead_letter_count(&self) -> usize {
    self.inner.dead_letters.lock().len()
  }

  pub fn clear_dead_letters(&self) {
    self.inner.dead_letters.lock().clear();
  }

  /// Jobs waiting to run, delayed ones included.
  pub fn pending_count(&self) -> usize {
    self.inner.pending.lock().len()
  }

  /// Ready, delayed, running and retrying jobs that still hold admission capacity.
  pub fn outstanding_count(&self) -> usize {
    self.inner.budget.usage().0
  }

  /// Serialized payload bytes held by outstanding jobs, counted once per job.
  pub fn outstanding_payload_bytes(&self) -> usize {
    self.inner.budget.usage().1
  }

  pub fn dead_letter_payload_bytes(&self) -> usize {
    self.inner.dead_letters.lock().bytes()
  }

  /// Dead letters discarded by retention limits since the queue was built.
  pub fn dead_letter_evictions(&self) -> u64 {
    self.inner.dead_letters.lock().evictions()
  }

  pub fn inflight_count(&self) -> u64 {
    self.inner.inflight.load(Ordering::SeqCst)
  }
}

impl Default for Queue {
  fn default() -> Self {
    Self::new()
  }
}
