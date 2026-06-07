//! Queue runtime: builder and lifecycle handles.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use parking_lot::Mutex;
use scc::HashMap as SccHashMap;
use tokio::sync::Notify;

use super::DeadJob;
use super::Job;
use super::QueueBuilder;
use super::QueueError;
use super::RetryPolicy;
#[cfg(feature = "signals")]
use super::signal_ids;
use super::worker::worker_loop;
#[cfg(feature = "signals")]
use crate::signals::Signal;
#[cfg(feature = "signals")]
use crate::signals::SignalArbiter;

pub(crate) struct PendingJob {
  pub(crate) id: u64,
  pub(crate) name: String,
  pub(crate) payload: Vec<u8>,
  pub(crate) attempt: u32,
  pub(crate) run_after: Option<Instant>,
  pub(crate) dedup_key: Option<String>,
}

pub(crate) type BoxHandler =
  Arc<dyn Fn(Job) -> Pin<Box<dyn Future<Output = Result<(), QueueError>> + Send>> + Send + Sync>;

pub(crate) struct QueueInner {
  /// Pending jobs waiting to be processed.
  pub(crate) pending: Mutex<VecDeque<PendingJob>>,
  /// Registered job handlers by name.
  pub(crate) handlers: SccHashMap<String, BoxHandler>,
  /// Dead letter queue.
  ///
  /// Stored as `Vec<Arc<DeadJob>>` so the [`Queue::dead_letters_arc`]
  /// snapshot only clones the outer `Vec` plus cheap atomic-refcount bumps
  /// per entry, rather than deep-copying every `payload` / `name` / `error`
  /// string. The owned [`Queue::dead_letters`] accessor still pays the deep
  /// clone for API compatibility.
  pub(crate) dead_letters: Mutex<Vec<Arc<DeadJob>>>,
  /// Notify workers when new jobs arrive.
  pub(crate) notify: Notify,
  /// Monotonically increasing job ID counter.
  pub(crate) next_id: AtomicU64,
  /// Number of worker tasks.
  pub(crate) num_workers: usize,
  /// Retry policy.
  pub(crate) retry_policy: RetryPolicy,
  /// Whether the queue has been shut down.
  pub(crate) shutdown: AtomicBool,
  /// Track in-flight jobs for graceful shutdown.
  pub(crate) inflight: AtomicU64,
  /// Notify when inflight reaches 0.
  pub(crate) drain_notify: Notify,
}

/// An in-memory background job queue.
///
/// Create via [`Queue::builder()`] or [`Queue::new()`].
/// Register handlers with [`register()`](Queue::register), then push jobs
/// with [`push()`](Queue::push) or [`push_delayed()`](Queue::push_delayed).
///
/// The queue must be started with [`start()`](Queue::start) to spawn
/// background worker tasks that process jobs.
#[derive(Clone)]
pub struct Queue {
  pub(crate) inner: Arc<QueueInner>,
}

impl Queue {
  /// Create a queue with default settings (4 workers, no retries).
  pub fn new() -> Self {
    Self::builder().build()
  }

  /// Create a builder for customizing the queue.
  pub fn builder() -> QueueBuilder {
    QueueBuilder {
      workers: 4,
      retry: RetryPolicy::default(),
    }
  }

  /// Register a named job handler.
  ///
  /// The handler receives a [`Job`] and returns `Result<(), QueueError>`.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// queue.register("process_order", |job: Job| async move {
  ///     let order_id: u64 = job.deserialize()?;
  ///     // process the order ...
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
    let _ = self.inner.handlers.insert_sync(name, handler);
  }

  /// Push a job for immediate execution.
  ///
  /// The payload is serialized to JSON. Returns the job ID.
  pub async fn push(
    &self,
    name: impl Into<String>,
    payload: &(impl serde::Serialize + ?Sized),
  ) -> Result<u64, QueueError> {
    self.push_inner(name.into(), payload, None)
  }

  /// Push a job for delayed execution.
  ///
  /// The job will not be picked up by a worker until `delay` has elapsed.
  pub async fn push_delayed(
    &self,
    name: impl Into<String>,
    payload: &(impl serde::Serialize + ?Sized),
    delay: Duration,
  ) -> Result<u64, QueueError> {
    self.push_inner(name.into(), payload, Some(Instant::now() + delay))
  }

  /// Push with a dedup key — the job is queued at most once concurrently.
  ///
  /// If a job with the same `dedup_key` is currently in `pending`, this is a
  /// no-op and the existing id is returned. Useful for idempotent triggers
  /// (e.g. flush a cache only once per minute regardless of how many requests
  /// arrived). The dedup window ends when the job is picked up.
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

    // Hold the pending lock across the check-and-insert so two concurrent
    // `push_dedup` callers cannot both observe "no duplicate" and then both
    // enqueue their own copy of the job. Re-check `shutdown` inside the lock
    // so a concurrent `shutdown()` (which itself grabs this lock around the
    // flag flip) cannot slip in between the early check above and the push.
    let id = {
      let mut pending = self.inner.pending.lock();
      if self.inner.shutdown.load(Ordering::SeqCst) {
        return Err(QueueError::Shutdown);
      }
      for j in pending.iter() {
        if j.dedup_key.as_deref() == Some(key.as_str()) {
          return Ok(j.id);
        }
      }
      let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
      pending.push_back(PendingJob {
        id,
        name,
        payload: bytes,
        attempt: 0,
        run_after: None,
        dedup_key: Some(key),
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
      // Re-check shutdown inside the lock — `shutdown()` flips the flag under
      // the same lock, so this turns the check-and-push into an atomic test
      // that cannot race with concurrent shutdown.
      if self.inner.shutdown.load(Ordering::SeqCst) {
        return Err(QueueError::Shutdown);
      }
      pending.push_back(PendingJob {
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
      // Best-effort fire-and-forget; the push API is sync for ergonomics.
      // Both runtimes need a spawn — previously the compio branch silently
      // dropped the arbiter future, so queue signals never fired under
      // io_uring.
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

  /// Start background worker tasks.
  ///
  /// This spawns `workers` number of tokio tasks that process jobs from the queue.
  /// Must be called once before pushing jobs.
  #[cfg(not(feature = "compio"))]
  pub fn start(&self) {
    for _ in 0..self.inner.num_workers {
      let inner = self.inner.clone();
      tokio::spawn(async move { worker_loop(inner).await });
    }
    tracing::debug!("Queue started with {} workers", self.inner.num_workers);
  }

  /// Start background worker tasks (compio runtime).
  #[cfg(feature = "compio")]
  pub fn start(&self) {
    for _ in 0..self.inner.num_workers {
      let inner = self.inner.clone();
      compio::runtime::spawn(async move { worker_loop(inner).await }).detach();
    }
    tracing::debug!("Queue started with {} workers", self.inner.num_workers);
  }

  /// Gracefully shut down the queue.
  ///
  /// Stops accepting new jobs and waits for in-flight jobs to complete
  /// (up to the given timeout).
  pub async fn shutdown(&self, timeout: Duration) {
    // Acquire the pending lock before flipping the flag so any concurrent
    // `push_inner` / `push_dedup` (which re-check `shutdown` while holding
    // the same lock) reliably observes the flip and rejects with
    // `QueueError::Shutdown` instead of silently enqueuing a job into a
    // queue whose workers are about to exit.
    {
      let _guard = self.inner.pending.lock();
      self.inner.shutdown.store(true, Ordering::SeqCst);
    }
    // Wake all workers so they see the shutdown flag.
    //
    // `Notify::notify_one` only stores ONE pending permit — sequential calls
    // collapse onto non-parked workers, so this loop only reliably wakes the
    // workers that happened to be parked at the moment of the first call.
    // Any worker mid-job (most of them, in practice) wouldn't observe the
    // wake; they only learned about shutdown via the 100ms park timeout.
    // `notify_waiters` permits *all* currently-parked waiters at once, which
    // is the desired shutdown semantics.
    self.inner.notify.notify_waiters();

    if self.inner.inflight.load(Ordering::SeqCst) > 0 {
      #[cfg(not(feature = "compio"))]
      {
        let _ = tokio::time::timeout(timeout, self.inner.drain_notify.notified()).await;
      }
      #[cfg(feature = "compio")]
      {
        let drain = std::pin::pin!(self.inner.drain_notify.notified());
        let sleep = std::pin::pin!(compio::time::sleep(timeout));
        let _ = futures_util::future::select(drain, sleep).await;
      }
    }

    tracing::debug!("Queue shut down");
  }

  /// Returns a snapshot of jobs in the dead letter queue.
  ///
  /// Allocates a new `Vec<DeadJob>` and deep-clones every entry. For
  /// monitoring code that just iterates entries read-only, prefer
  /// [`Self::dead_letters_arc`] (atomic refcount per entry, no payload
  /// copies) or [`Self::dead_letter_count`] (no allocation at all).
  pub fn dead_letters(&self) -> Vec<DeadJob> {
    self
      .inner
      .dead_letters
      .lock()
      .iter()
      .map(|j| (**j).clone())
      .collect()
  }

  /// Returns a cheap snapshot of the dead letter queue.
  ///
  /// Each entry is shared via `Arc` rather than deep-cloned, so this is
  /// suitable for emitting from metrics endpoints or hot paths. Returned
  /// `Arc<DeadJob>` handles remain valid even if [`Self::clear_dead_letters`]
  /// is called concurrently.
  pub fn dead_letters_arc(&self) -> Vec<Arc<DeadJob>> {
    self.inner.dead_letters.lock().clone()
  }

  /// Returns the number of jobs currently in the dead letter queue.
  pub fn dead_letter_count(&self) -> usize {
    self.inner.dead_letters.lock().len()
  }

  /// Clear all dead letters.
  pub fn clear_dead_letters(&self) {
    self.inner.dead_letters.lock().clear();
  }

  /// Returns the number of pending jobs.
  pub fn pending_count(&self) -> usize {
    self.inner.pending.lock().len()
  }

  /// Returns the number of currently in-flight jobs.
  pub fn inflight_count(&self) -> u64 {
    self.inner.inflight.load(Ordering::SeqCst)
  }
}

impl Default for Queue {
  fn default() -> Self {
    Self::new()
  }
}
