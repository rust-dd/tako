use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use parking_lot::Mutex;
use tokio::sync::Notify;

use super::DeadJob;
use super::Job;
use super::QueueBuilder;
use super::QueueError;
use super::QueueHandle;
use super::RetryPolicy;
use super::lifecycle::Lifecycle;
use super::lifecycle::QueueOwner;
use super::pending::PendingJob;
use super::pending::PendingQueue;
#[cfg(feature = "signals")]
use super::signal_ids;
#[cfg(feature = "signals")]
use crate::signals::Signal;
#[cfg(feature = "signals")]
use crate::signals::SignalArbiter;

pub(crate) type BoxHandler =
  Arc<dyn Fn(Job) -> Pin<Box<dyn Future<Output = Result<(), QueueError>> + Send>> + Send + Sync>;

pub(crate) struct QueueInner {
  pub(crate) pending: Mutex<PendingQueue>,
  pub(crate) handlers: Mutex<HashMap<String, BoxHandler>>,
  /// `Arc` entries let [`Queue::dead_letters`] snapshot without copying payloads.
  pub(crate) dead_letters: Mutex<Vec<Arc<DeadJob>>>,
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

  /// Queues a job with `payload` serialized to JSON and returns its id.
  #[allow(
    clippy::unused_async_trait_impl,
    reason = "public async API that enqueues when polled"
  )]
  pub async fn push(
    &self,
    name: impl Into<String>,
    payload: &(impl serde::Serialize + ?Sized),
  ) -> Result<u64, QueueError> {
    self.push_inner(name.into(), payload, None)
  }

  /// Like [`push`](Self::push), but no worker picks the job up before `delay` elapses.
  #[allow(
    clippy::unused_async_trait_impl,
    reason = "public async API that starts the delay when polled"
  )]
  pub async fn push_delayed(
    &self,
    name: impl Into<String>,
    payload: &(impl serde::Serialize + ?Sized),
    delay: Duration,
  ) -> Result<u64, QueueError> {
    self.push_inner(name.into(), payload, Some(Instant::now() + delay))
  }

  /// Like [`push`](Self::push), but while a job with `dedup_key` waits to be picked up it
  /// returns that job's id instead of queueing another.
  #[allow(
    clippy::unused_async_trait_impl,
    reason = "public async API that deduplicates when polled"
  )]
  pub async fn push_dedup(
    &self,
    name: impl Into<String>,
    payload: &(impl serde::Serialize + ?Sized),
    dedup_key: impl Into<String>,
  ) -> Result<u64, QueueError> {
    if self.inner.shutdown.load(Ordering::SeqCst) {
      return Err(QueueError::Shutdown);
    }
    let key = dedup_key.into();
    let name = name.into();
    let bytes =
      serde_json::to_vec(payload).map_err(|e| QueueError::SerializeError(e.to_string()))?;

    // Deduplication and shutdown share the lock with enqueueing.
    let id = {
      let mut pending = self.inner.pending.lock();
      if self.inner.shutdown.load(Ordering::SeqCst) {
        return Err(QueueError::Shutdown);
      }
      if let Some(id) = pending.existing_id(&key) {
        return Ok(id);
      }
      let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
      pending.push(PendingJob {
        id,
        name,
        payload: bytes,
        attempt: 0,
        run_after: None,
        dedup_key: Some(key.into()),
      });
      id
    };

    self.inner.notify.notify_one();
    Ok(id)
  }

  fn push_inner(
    &self,
    name: String,
    payload: &(impl serde::Serialize + ?Sized),
    run_after: Option<Instant>,
  ) -> Result<u64, QueueError> {
    if self.inner.shutdown.load(Ordering::SeqCst) {
      return Err(QueueError::Shutdown);
    }

    let bytes =
      serde_json::to_vec(payload).map_err(|e| QueueError::SerializeError(e.to_string()))?;

    let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);

    #[cfg(feature = "signals")]
    let job_name = name.clone();
    {
      let mut pending = self.inner.pending.lock();
      // Shutdown flips the flag under this same lock.
      if self.inner.shutdown.load(Ordering::SeqCst) {
        return Err(QueueError::Shutdown);
      }
      pending.push(PendingJob {
        id,
        name,
        payload: bytes,
        attempt: 0,
        run_after,
        dedup_key: None,
      });
    }

    self.inner.notify.notify_one();
    #[cfg(feature = "signals")]
    {
      let arbiter = SignalArbiter::emit_app(
        Signal::with_capacity(signal_ids::QUEUE_JOB_QUEUED, 2)
          .meta("name", job_name)
          .meta("id", id.to_string()),
      );
      // Signal delivery must not block enqueueing on either runtime.
      #[cfg(not(feature = "compio"))]
      {
        tokio::spawn(arbiter);
      }
      #[cfg(feature = "compio")]
      {
        compio::runtime::spawn(arbiter).detach();
      }
    }
    Ok(id)
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
    self.inner.dead_letters.lock().clone()
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

  pub fn inflight_count(&self) -> u64 {
    self.inner.inflight.load(Ordering::SeqCst)
  }
}

impl Default for Queue {
  fn default() -> Self {
    Self::new()
  }
}
