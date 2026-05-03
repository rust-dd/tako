//! In-memory background job queue with named queues, retry policies, and dead letter support.
//!
//! Provides a lightweight task queue for deferring work to background workers —
//! useful for sending emails, webhooks, async processing, etc.
//!
//! # Features
//!
//! - **Named queues** — separate logical channels (e.g. `"email"`, `"webhook"`)
//! - **Configurable workers** — per-queue concurrency limit
//! - **Retry policy** — fixed or exponential backoff with max attempts
//! - **Delayed jobs** — schedule execution after a duration
//! - **Dead letter queue** — failed jobs stored for inspection
//! - **Graceful shutdown** — drain in-flight jobs before exit
//!
//! # Examples
//!
//! ```rust,no_run
//! use tako::queue::{Queue, RetryPolicy, Job};
//! use std::time::Duration;
//!
//! # async fn example() {
//! let queue = Queue::builder()
//!     .workers(4)
//!     .retry(RetryPolicy::exponential(3, Duration::from_secs(1)))
//!     .build();
//!
//! queue.register("send_email", |job: Job| async move {
//!     let to: String = job.deserialize()?;
//!     println!("Sending email to {to}");
//!     Ok(())
//! });
//!
//! queue.push("send_email", &"user@example.com").await.unwrap();
//! # }
//! ```

/// Pluggable queue backend abstraction (v2). The bundled `Queue` keeps its
/// in-process semantics; opt into a remote broker via [`backend::QueueBackend`].
pub mod backend;

/// Cron scheduling on top of `QueueBackend` (opt-in via `queue-cron` feature).
#[cfg(feature = "queue-cron")]
#[cfg_attr(docsrs, doc(cfg(feature = "queue-cron")))]
pub mod cron;

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

#[cfg(feature = "signals")]
use crate::signals::Signal;
#[cfg(feature = "signals")]
use crate::signals::SignalArbiter;

/// Well-known queue signal ids.
#[cfg(feature = "signals")]
pub mod signal_ids {
  pub const QUEUE_JOB_QUEUED: &str = "queue.job.queued";
  pub const QUEUE_JOB_STARTED: &str = "queue.job.started";
  pub const QUEUE_JOB_COMPLETED: &str = "queue.job.completed";
  pub const QUEUE_JOB_FAILED: &str = "queue.job.failed";
  pub const QUEUE_JOB_RETRYING: &str = "queue.job.retrying";
  pub const QUEUE_JOB_DEAD_LETTER: &str = "queue.job.dead_letter";
}

#[cfg(feature = "signals")]
async fn emit_queue_signal(id: &'static str, name: &str, job_id: u64, attempt: u32) {
  SignalArbiter::emit_app(
    Signal::with_capacity(id, 3)
      .meta("name", name)
      .meta("id", job_id.to_string())
      .meta("attempt", attempt.to_string()),
  )
  .await;
}

/// Error type for queue operations.
#[derive(Debug)]
pub enum QueueError {
  /// No handler registered for the given job name.
  UnknownJob(String),
  /// Failed to serialize job payload.
  SerializeError(String),
  /// The job handler returned an error.
  HandlerError(String),
  /// Queue has been shut down.
  Shutdown,
}

impl std::fmt::Display for QueueError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::UnknownJob(name) => write!(f, "no handler registered for job '{name}'"),
      Self::SerializeError(e) => write!(f, "failed to serialize job payload: {e}"),
      Self::HandlerError(e) => write!(f, "job handler error: {e}"),
      Self::Shutdown => write!(f, "queue has been shut down"),
    }
  }
}

impl std::error::Error for QueueError {}

/// Retry policy for failed jobs.
#[derive(Debug, Clone, Default)]
pub enum RetryPolicy {
  /// No retries — failed jobs go straight to the dead letter queue.
  #[default]
  None,
  /// Fixed delay between retries.
  Fixed {
    /// Maximum number of retry attempts.
    max_retries: u32,
    /// Delay between each retry.
    delay: Duration,
  },
  /// Exponential backoff between retries.
  Exponential {
    /// Maximum number of retry attempts.
    max_retries: u32,
    /// Initial delay (doubled on each retry).
    base_delay: Duration,
  },
}

impl RetryPolicy {
  /// Create a fixed-delay retry policy.
  pub fn fixed(max_retries: u32, delay: Duration) -> Self {
    Self::Fixed { max_retries, delay }
  }

  /// Create an exponential-backoff retry policy.
  pub fn exponential(max_retries: u32, base_delay: Duration) -> Self {
    Self::Exponential {
      max_retries,
      base_delay,
    }
  }

  fn max_retries(&self) -> u32 {
    match self {
      Self::None => 0,
      Self::Fixed { max_retries, .. } | Self::Exponential { max_retries, .. } => *max_retries,
    }
  }

  fn delay_for_attempt(&self, attempt: u32) -> Duration {
    match self {
      Self::None => Duration::ZERO,
      Self::Fixed { delay, .. } => *delay,
      Self::Exponential { base_delay, .. } => *base_delay * 2u32.saturating_pow(attempt),
    }
  }
}

/// A job passed to a handler function.
///
/// Contains the serialized payload and metadata about the job.
pub struct Job {
  /// The raw JSON payload.
  pub(crate) payload: Vec<u8>,
  /// Job name (the key it was registered under).
  pub name: String,
  /// Current attempt number (0-based).
  pub attempt: u32,
  /// Unique job ID.
  pub id: u64,
}

impl Job {
  /// Deserialize the job payload into the expected type.
  pub fn deserialize<T: serde::de::DeserializeOwned>(&self) -> Result<T, QueueError> {
    serde_json::from_slice(&self.payload).map_err(|e| QueueError::HandlerError(e.to_string()))
  }

  /// Access the raw payload bytes.
  pub fn raw_payload(&self) -> &[u8] {
    &self.payload
  }
}

/// A failed job stored in the dead letter queue.
#[derive(Debug, Clone)]
pub struct DeadJob {
  /// Unique job ID.
  pub id: u64,
  /// Job name.
  pub name: String,
  /// Raw payload.
  pub payload: Vec<u8>,
  /// Number of attempts made.
  pub attempts: u32,
  /// The final error message.
  pub error: String,
  /// When the job was moved to the DLQ.
  pub failed_at: Instant,
}

struct PendingJob {
  id: u64,
  name: String,
  payload: Vec<u8>,
  attempt: u32,
  run_after: Option<Instant>,
  dedup_key: Option<String>,
}

type BoxHandler =
  Arc<dyn Fn(Job) -> Pin<Box<dyn Future<Output = Result<(), QueueError>> + Send>> + Send + Sync>;

struct QueueInner {
  /// Pending jobs waiting to be processed.
  pending: Mutex<VecDeque<PendingJob>>,
  /// Registered job handlers by name.
  handlers: SccHashMap<String, BoxHandler>,
  /// Dead letter queue.
  dead_letters: Mutex<Vec<DeadJob>>,
  /// Notify workers when new jobs arrive.
  notify: Notify,
  /// Monotonically increasing job ID counter.
  next_id: AtomicU64,
  /// Number of worker tasks.
  num_workers: usize,
  /// Retry policy.
  retry_policy: RetryPolicy,
  /// Whether the queue has been shut down.
  shutdown: AtomicBool,
  /// Track in-flight jobs for graceful shutdown.
  inflight: AtomicU64,
  /// Notify when inflight reaches 0.
  drain_notify: Notify,
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
  inner: Arc<QueueInner>,
}

/// Builder for configuring a [`Queue`].
pub struct QueueBuilder {
  workers: usize,
  retry: RetryPolicy,
}

impl QueueBuilder {
  /// Set the number of worker tasks (default: 4).
  pub fn workers(mut self, n: usize) -> Self {
    self.workers = n.max(1);
    self
  }

  /// Set the retry policy for failed jobs.
  pub fn retry(mut self, policy: RetryPolicy) -> Self {
    self.retry = policy;
    self
  }

  /// Build the queue. Call [`Queue::start()`] to begin processing.
  pub fn build(self) -> Queue {
    Queue {
      inner: Arc::new(QueueInner {
        pending: Mutex::new(VecDeque::new()),
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
    let key = dedup_key.into();
    {
      let pending = self.inner.pending.lock();
      for j in pending.iter() {
        if j.dedup_key.as_deref() == Some(key.as_str()) {
          return Ok(j.id);
        }
      }
    }
    let name = name.into();
    self.push_inner_keyed(name, payload, None, Some(key))
  }

  fn push_inner_keyed(
    &self,
    name: String,
    payload: &(impl serde::Serialize + ?Sized),
    run_after: Option<Instant>,
    dedup_key: Option<String>,
  ) -> Result<u64, QueueError> {
    if self.inner.shutdown.load(Ordering::SeqCst) {
      return Err(QueueError::Shutdown);
    }

    let bytes =
      serde_json::to_vec(payload).map_err(|e| QueueError::SerializeError(e.to_string()))?;

    let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);

    self.inner.pending.lock().push_back(PendingJob {
      id,
      name,
      payload: bytes,
      attempt: 0,
      run_after,
      dedup_key,
    });

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
    self.inner.pending.lock().push_back(PendingJob {
      id,
      name,
      payload: bytes,
      attempt: 0,
      run_after,
      dedup_key: None,
    });

    self.inner.notify.notify_one();
    #[cfg(feature = "signals")]
    {
      let arbiter = SignalArbiter::emit_app(
        Signal::with_capacity(signal_ids::QUEUE_JOB_QUEUED, 2)
          .meta("name", job_name)
          .meta("id", id.to_string()),
      );
      // Best-effort fire-and-forget; the push API is sync for ergonomics.
      tokio::spawn(arbiter);
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
    self.inner.shutdown.store(true, Ordering::SeqCst);
    // Wake all workers so they see the shutdown flag
    for _ in 0..self.inner.num_workers {
      self.inner.notify.notify_one();
    }

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
  pub fn dead_letters(&self) -> Vec<DeadJob> {
    self.inner.dead_letters.lock().clone()
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

async fn worker_loop(inner: Arc<QueueInner>) {
  loop {
    // Wait for notification or check periodically for delayed jobs
    #[cfg(not(feature = "compio"))]
    {
      let _ = tokio::time::timeout(Duration::from_millis(100), inner.notify.notified()).await;
    }
    #[cfg(feature = "compio")]
    {
      let notified = std::pin::pin!(inner.notify.notified());
      let sleep = std::pin::pin!(compio::time::sleep(Duration::from_millis(100)));
      let _ = futures_util::future::select(notified, sleep).await;
    }

    if inner.shutdown.load(Ordering::SeqCst) && inner.pending.lock().is_empty() {
      break;
    }

    // Try to pick up a job
    let job = {
      let mut pending = inner.pending.lock();
      let now = Instant::now();

      // Find the first job that's ready to run
      let pos = pending.iter().position(|j| match j.run_after {
        Some(t) => now >= t,
        None => true,
      });

      pos.and_then(|i| pending.remove(i))
    };

    let Some(pending_job) = job else {
      continue;
    };

    // Look up handler
    let handler = inner
      .handlers
      .get_async(&pending_job.name)
      .await
      .map(|e| e.get().clone());

    let Some(handler) = handler else {
      tracing::warn!("No handler for job '{}', moving to DLQ", pending_job.name);
      #[cfg(feature = "signals")]
      emit_queue_signal(
        signal_ids::QUEUE_JOB_DEAD_LETTER,
        &pending_job.name,
        pending_job.id,
        pending_job.attempt + 1,
      )
      .await;
      inner.dead_letters.lock().push(DeadJob {
        id: pending_job.id,
        name: pending_job.name,
        payload: pending_job.payload,
        attempts: pending_job.attempt + 1,
        error: "no handler registered".into(),
        failed_at: Instant::now(),
      });
      continue;
    };

    inner.inflight.fetch_add(1, Ordering::SeqCst);

    #[cfg(feature = "signals")]
    emit_queue_signal(
      signal_ids::QUEUE_JOB_STARTED,
      &pending_job.name,
      pending_job.id,
      pending_job.attempt,
    )
    .await;

    let job = Job {
      payload: pending_job.payload.clone(),
      name: pending_job.name.clone(),
      attempt: pending_job.attempt,
      id: pending_job.id,
    };

    let result = handler(job).await;

    #[cfg(feature = "signals")]
    if result.is_ok() {
      emit_queue_signal(
        signal_ids::QUEUE_JOB_COMPLETED,
        &pending_job.name,
        pending_job.id,
        pending_job.attempt,
      )
      .await;
    } else {
      emit_queue_signal(
        signal_ids::QUEUE_JOB_FAILED,
        &pending_job.name,
        pending_job.id,
        pending_job.attempt,
      )
      .await;
    }

    if let Err(e) = result {
      let max_retries = inner.retry_policy.max_retries();

      if pending_job.attempt < max_retries {
        let next_attempt = pending_job.attempt + 1;
        let delay = inner.retry_policy.delay_for_attempt(pending_job.attempt);

        tracing::debug!(
          "Job '{}' (id={}) failed (attempt {}/{}), retrying in {:?}",
          pending_job.name,
          pending_job.id,
          next_attempt,
          max_retries,
          delay
        );

        #[cfg(feature = "signals")]
        emit_queue_signal(
          signal_ids::QUEUE_JOB_RETRYING,
          &pending_job.name,
          pending_job.id,
          next_attempt,
        )
        .await;

        inner.pending.lock().push_back(PendingJob {
          id: pending_job.id,
          name: pending_job.name,
          payload: pending_job.payload,
          attempt: next_attempt,
          run_after: Some(Instant::now() + delay),
          dedup_key: None,
        });

        inner.notify.notify_one();
      } else {
        tracing::warn!(
          "Job '{}' (id={}) exhausted {} retries, moving to DLQ: {}",
          pending_job.name,
          pending_job.id,
          max_retries,
          e
        );

        #[cfg(feature = "signals")]
        emit_queue_signal(
          signal_ids::QUEUE_JOB_DEAD_LETTER,
          &pending_job.name,
          pending_job.id,
          pending_job.attempt + 1,
        )
        .await;

        inner.dead_letters.lock().push(DeadJob {
          id: pending_job.id,
          name: pending_job.name,
          payload: pending_job.payload,
          attempts: pending_job.attempt + 1,
          error: e.to_string(),
          failed_at: Instant::now(),
        });
      }
    }

    let prev = inner.inflight.fetch_sub(1, Ordering::SeqCst);
    if prev == 1 && inner.shutdown.load(Ordering::SeqCst) {
      inner.drain_notify.notify_one();
    }
  }
}
