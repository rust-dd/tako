//! Pluggable queue backend abstraction.
//!
//! `QueueBackend` is the v2 trait that lets a `Queue` swap its in-process
//! storage for an external broker (Redis, Postgres, NATS, …). The bundled
//! [`MemoryBackend`](crate::queue::backend::MemoryBackend) keeps the existing in-process `Queue` semantics behind
//! the same trait so consumer code can move to the trait at its own pace.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use parking_lot::Mutex;

/// Job identifier returned by [`QueueBackend::push`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobId(pub u64);

/// Per-push options.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct PushOptions {
  /// Idempotency / dedup key — duplicate pushes with the same key are coalesced.
  pub dedup_key: Option<String>,
  /// Schedule the job to execute no earlier than this instant.
  pub run_after: Option<Instant>,
  /// Optional named attempt count (for retries that re-push instead of mutating in place).
  pub attempt: u32,
}

/// Reserved job returned by [`QueueBackend::reserve`].
#[derive(Debug, Clone)]
pub struct ReservedJob {
  pub id: JobId,
  pub queue: String,
  pub payload: Vec<u8>,
  pub attempt: u32,
}

/// Queue backend trait — async by design, since real backends are remote.
#[async_trait]
pub trait QueueBackend: Send + Sync + 'static {
  /// Push a payload onto a named queue, returning the job id.
  async fn push(
    &self,
    queue: &str,
    payload: &[u8],
    opts: PushOptions,
  ) -> Result<JobId, BackendError>;

  /// Reserve the next ready job from a queue (`FIFO`, `ready_at` <= now).
  async fn reserve(&self, queue: &str) -> Result<Option<ReservedJob>, BackendError>;

  /// Mark a reserved job complete — no retries, no DLQ entry.
  async fn complete(&self, id: JobId) -> Result<(), BackendError>;

  /// Mark a reserved job failed; if `retry_at` is set, requeue for that time.
  async fn fail(&self, id: JobId, retry_at: Option<Instant>) -> Result<(), BackendError>;

  /// Move a job to the dead-letter queue (terminal failure).
  async fn dead_letter(&self, id: JobId) -> Result<(), BackendError>;
}

/// Backend-level error.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum BackendError {
  /// The transport (Redis, Postgres, …) returned an error.
  Transport(String),
  /// The job referenced by `JobId` does not exist.
  NotFound,
}

impl std::fmt::Display for BackendError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Transport(e) => write!(f, "queue transport error: {e}"),
      Self::NotFound => write!(f, "job not found"),
    }
  }
}

impl std::error::Error for BackendError {}

/// In-process backend keeping job state in a single `Mutex<Vec<…>>`. Drop-in
/// replacement for the bundled `Queue` storage; suitable for tests and
/// single-node deployments. Replace with a remote backend for multi-pod use.
///
/// **Memory bound**: the pending queue is capped at [`MemoryBackend::max_pending`]
/// (`None` = unlimited, default). Configure via [`MemoryBackend::with_max_pending`]
/// to protect against unbounded growth when producers outpace consumers.
#[derive(Default)]
pub struct MemoryBackend {
  inner: Arc<Mutex<MemoryInner>>,
  next_id: std::sync::atomic::AtomicU64,
  max_pending: Option<usize>,
}

#[derive(Default)]
struct MemoryInner {
  pending: Vec<MemoryJob>,
  reserved: Vec<MemoryJob>,
  dlq: Vec<MemoryJob>,
  /// Active dedup keys.
  ///
  /// **Window semantics**: a key is inserted by [`MemoryBackend::push`] when
  /// the caller supplied `PushOptions::dedup_key`, and is removed only by
  /// [`MemoryBackend::complete`] (success) or [`MemoryBackend::dead_letter`]
  /// (terminal failure). [`MemoryBackend::fail`] (transient retry) deliberately
  /// keeps the key alive so a duplicate push during the retry window is
  /// idempotent — the original job is still going to run.
  ///
  /// Callers that need a shorter window can call
  /// [`MemoryBackend::purge_dedup_key`] explicitly.
  dedup: std::collections::HashSet<String>,
}

#[derive(Clone)]
struct MemoryJob {
  id: JobId,
  queue: String,
  payload: Vec<u8>,
  attempt: u32,
  run_after: Option<Instant>,
  dedup_key: Option<String>,
}

impl MemoryBackend {
  pub fn new() -> Self {
    Self::default()
  }

  /// Cap the pending vector at `max` jobs. Subsequent
  /// [`MemoryBackend::push`] calls that would exceed the cap return
  /// [`BackendError::Transport`] with the message `queue full`.
  ///
  /// Use this to bound memory growth when a misconfigured retry loop or
  /// runaway producer would otherwise fill the heap.
  #[must_use]
  pub fn with_max_pending(mut self, max: usize) -> Self {
    self.max_pending = Some(max);
    self
  }
}

#[async_trait]
impl QueueBackend for MemoryBackend {
  async fn push(
    &self,
    queue: &str,
    payload: &[u8],
    opts: PushOptions,
  ) -> Result<JobId, BackendError> {
    let mut inner = self.inner.lock();
    if let Some(cap) = self.max_pending
      && inner.pending.len() >= cap
    {
      return Err(BackendError::Transport("queue full".into()));
    }
    if let Some(key) = opts.dedup_key.as_ref()
      && !inner.dedup.insert(key.clone())
    {
      // Treat as a successful idempotent no-op; report a synthetic id.
      return Ok(JobId(0));
    }
    let id = JobId(
      self
        .next_id
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        + 1,
    );
    inner.pending.push(MemoryJob {
      id,
      queue: queue.to_string(),
      payload: payload.to_vec(),
      attempt: opts.attempt,
      run_after: opts.run_after,
      dedup_key: opts.dedup_key,
    });
    Ok(id)
  }

  async fn reserve(&self, queue: &str) -> Result<Option<ReservedJob>, BackendError> {
    let mut inner = self.inner.lock();
    let now = Instant::now();
    let pos = inner
      .pending
      .iter()
      .position(|j| j.queue == queue && j.run_after.is_none_or(|t| now >= t));
    let Some(idx) = pos else {
      return Ok(None);
    };
    let job = inner.pending.remove(idx);
    let reserved = ReservedJob {
      id: job.id,
      queue: job.queue.clone(),
      payload: job.payload.clone(),
      attempt: job.attempt,
    };
    inner.reserved.push(job);
    Ok(Some(reserved))
  }

  async fn complete(&self, id: JobId) -> Result<(), BackendError> {
    let mut inner = self.inner.lock();
    let pos = inner.reserved.iter().position(|j| j.id == id);
    let Some(idx) = pos else {
      return Err(BackendError::NotFound);
    };
    let job = inner.reserved.remove(idx);
    if let Some(key) = job.dedup_key {
      inner.dedup.remove(&key);
    }
    Ok(())
  }

  async fn fail(&self, id: JobId, retry_at: Option<Instant>) -> Result<(), BackendError> {
    let mut inner = self.inner.lock();
    let pos = inner.reserved.iter().position(|j| j.id == id);
    let Some(idx) = pos else {
      return Err(BackendError::NotFound);
    };
    let mut job = inner.reserved.remove(idx);
    job.attempt = job.attempt.saturating_add(1);
    job.run_after = retry_at;
    inner.pending.push(job);
    Ok(())
  }

  async fn dead_letter(&self, id: JobId) -> Result<(), BackendError> {
    let mut inner = self.inner.lock();
    let pos = inner.reserved.iter().position(|j| j.id == id);
    let Some(idx) = pos else {
      return Err(BackendError::NotFound);
    };
    let job = inner.reserved.remove(idx);
    if let Some(key) = job.dedup_key.as_ref() {
      inner.dedup.remove(key);
    }
    inner.dlq.push(job);
    Ok(())
  }
}

impl MemoryBackend {
  /// Snapshot of dead-letter jobs.
  pub fn dead_letters(&self) -> Vec<(JobId, String, Vec<u8>, u32)> {
    self
      .inner
      .lock()
      .dlq
      .iter()
      .map(|j| (j.id, j.queue.clone(), j.payload.clone(), j.attempt))
      .collect()
  }

  /// Number of pending jobs across every queue.
  pub fn pending_count(&self) -> usize {
    self.inner.lock().pending.len()
  }

  /// Number of currently reserved jobs.
  pub fn reserved_count(&self) -> usize {
    self.inner.lock().reserved.len()
  }

  /// Number of active dedup keys. Useful for diagnostics — if this grows
  /// unboundedly your jobs are not reaching `complete`/`dead_letter`.
  pub fn dedup_size(&self) -> usize {
    self.inner.lock().dedup.len()
  }

  /// Drop a single dedup key out of band. Returns `true` if the key was
  /// present. Use this when a long retry tail needs to be cut short and a
  /// fresh push (same key) should be allowed to enqueue immediately.
  pub fn purge_dedup_key(&self, key: &str) -> bool {
    self.inner.lock().dedup.remove(key)
  }
}
