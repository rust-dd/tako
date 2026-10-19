//! Trait for a job broker outside the process, plus the in-memory [`MemoryBackend`].
//! [`Queue`](super::Queue) keeps its own storage; the cron scheduler pushes through this trait.

use std::time::Instant;

use async_trait::async_trait;
use parking_lot::Mutex;

/// Job identifier returned by [`QueueBackend::push`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobId(pub u64);

#[derive(Debug, Clone, Default)]
pub struct PushOptions {
  /// A push whose key is still active is coalesced into the existing job.
  pub dedup_key: Option<String>,
  /// Earliest time the job may run.
  pub run_after: Option<Instant>,
  /// Attempt number to record, for retries that push the job again.
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

/// Job broker; async because real brokers are remote.
#[async_trait]
pub trait QueueBackend: Send + Sync + 'static {
  async fn push(
    &self,
    queue: &str,
    payload: &[u8],
    opts: PushOptions,
  ) -> Result<JobId, BackendError>;

  /// Takes the oldest job of `queue` whose `run_after` has passed.
  async fn reserve(&self, queue: &str) -> Result<Option<ReservedJob>, BackendError>;

  /// Marks a reserved job done: no retry, no dead letter.
  async fn complete(&self, id: JobId) -> Result<(), BackendError>;

  /// Requeues a reserved job to run at `retry_at`, or at once.
  async fn fail(&self, id: JobId, retry_at: Option<Instant>) -> Result<(), BackendError>;

  /// Moves a reserved job to the dead letter queue for good.
  async fn dead_letter(&self, id: JobId) -> Result<(), BackendError>;
}

#[derive(Debug, Clone)]
pub enum BackendError {
  /// The transport (Redis, Postgres, …) returned an error.
  Transport(String),
  /// No reserved job has this id.
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

/// In-process [`QueueBackend`] for tests and single-node use, unbounded unless
/// [`with_max_pending`](Self::with_max_pending) caps it.
#[derive(Default)]
pub struct MemoryBackend {
  inner: Mutex<MemoryInner>,
  next_id: std::sync::atomic::AtomicU64,
  max_pending: Option<usize>,
}

#[derive(Default)]
struct MemoryInner {
  pending: Vec<MemoryJob>,
  reserved: Vec<MemoryJob>,
  dlq: Vec<MemoryJob>,
  /// A key lives until `complete` or `dead_letter`, so a duplicate pushed while the job
  /// waits for a retry is still coalesced.
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

  /// Fails pushes with `Transport("queue full")` once `max` jobs are pending.
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
      // A coalesced push reports the placeholder id 0.
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
  /// Dead-lettered jobs as `(id, queue, payload, attempt)`.
  pub fn dead_letters(&self) -> Vec<(JobId, String, Vec<u8>, u32)> {
    self
      .inner
      .lock()
      .dlq
      .iter()
      .map(|j| (j.id, j.queue.clone(), j.payload.clone(), j.attempt))
      .collect()
  }

  /// Pending jobs across every queue.
  pub fn pending_count(&self) -> usize {
    self.inner.lock().pending.len()
  }

  pub fn reserved_count(&self) -> usize {
    self.inner.lock().reserved.len()
  }

  /// Active dedup keys; steady growth means jobs never reach `complete` or `dead_letter`.
  pub fn dedup_size(&self) -> usize {
    self.inner.lock().dedup.len()
  }

  /// Drops a dedup key so the next push with it enqueues; `true` if the key was active.
  pub fn purge_dedup_key(&self, key: &str) -> bool {
    self.inner.lock().dedup.remove(key)
  }
}
