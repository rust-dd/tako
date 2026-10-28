use std::io;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use super::CapacityExceeded;
use super::EnqueueError;
use super::Queue;
use super::QueueError;
use super::pending::PendingJob;

impl Queue {
  /// Capacity failures use `HandlerError`; [`Self::try_push`] returns a typed error.
  pub async fn push(
    &self,
    name: impl Into<String>,
    payload: &(impl serde::Serialize + ?Sized),
  ) -> Result<u64, QueueError> {
    self.try_push(name, payload).await.map_err(Into::into)
  }

  /// Queues a JSON payload without waiting for capacity to become available.
  #[allow(
    clippy::unused_async_trait_impl,
    reason = "enqueue only when the future is polled"
  )]
  pub async fn try_push(
    &self,
    name: impl Into<String>,
    payload: &(impl serde::Serialize + ?Sized),
  ) -> Result<u64, EnqueueError> {
    self.enqueue(name.into(), payload, None, None)
  }

  /// Like [`Self::push`], but no worker picks the job up before `delay` elapses.
  pub async fn push_delayed(
    &self,
    name: impl Into<String>,
    payload: &(impl serde::Serialize + ?Sized),
    delay: Duration,
  ) -> Result<u64, QueueError> {
    self
      .try_push_delayed(name, payload, delay)
      .await
      .map_err(Into::into)
  }

  /// Reserves capacity immediately for a job that becomes ready after `delay`.
  #[allow(
    clippy::unused_async_trait_impl,
    reason = "start the delay when the future is polled"
  )]
  pub async fn try_push_delayed(
    &self,
    name: impl Into<String>,
    payload: &(impl serde::Serialize + ?Sized),
    delay: Duration,
  ) -> Result<u64, EnqueueError> {
    let deadline = Instant::now()
      .checked_add(delay)
      .ok_or_else(|| QueueError::HandlerError("job delay exceeds the clock range".into()))?;
    self.enqueue(name.into(), payload, Some(deadline), None)
  }

  /// Returns an existing pending job's id when `dedup_key` matches it.
  pub async fn push_dedup(
    &self,
    name: impl Into<String>,
    payload: &(impl serde::Serialize + ?Sized),
    dedup_key: impl Into<String>,
  ) -> Result<u64, QueueError> {
    self
      .try_push_dedup(name, payload, dedup_key)
      .await
      .map_err(Into::into)
  }

  /// Pending duplicates do not consume capacity.
  #[allow(
    clippy::unused_async_trait_impl,
    reason = "deduplicate only when the future is polled"
  )]
  pub async fn try_push_dedup(
    &self,
    name: impl Into<String>,
    payload: &(impl serde::Serialize + ?Sized),
    dedup_key: impl Into<String>,
  ) -> Result<u64, EnqueueError> {
    self.enqueue(name.into(), payload, None, Some(dedup_key.into()))
  }

  fn enqueue(
    &self,
    name: String,
    payload: &(impl serde::Serialize + ?Sized),
    run_after: Option<Instant>,
    dedup_key: Option<String>,
  ) -> Result<u64, EnqueueError> {
    if self.inner.shutdown.load(Ordering::SeqCst) {
      return Err(QueueError::Shutdown.into());
    }
    if let Some(key) = &dedup_key {
      let pending = self.inner.pending.lock();
      if self.inner.shutdown.load(Ordering::SeqCst) {
        return Err(QueueError::Shutdown.into());
      }
      if let Some(id) = pending.existing_id(key) {
        return Ok(id);
      }
    }
    let bytes = serialize(payload, self.inner.budget.limits().payload_bytes)?;
    #[cfg(feature = "signals")]
    let job_name = name.clone();
    let id = {
      let mut pending = self.inner.pending.lock();
      if self.inner.shutdown.load(Ordering::SeqCst) {
        return Err(QueueError::Shutdown.into());
      }
      if let Some(id) = dedup_key.as_ref().and_then(|key| pending.existing_id(key)) {
        return Ok(id);
      }
      let permit = self.inner.budget.reserve(bytes.len())?;
      let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
      pending.push(PendingJob {
        id,
        name,
        payload: bytes,
        attempt: 0,
        run_after,
        dedup_key: dedup_key.map(Into::into),
        _permit: Some(permit),
      });
      id
    };
    self.inner.notify.notify_one();
    #[cfg(feature = "signals")]
    {
      use crate::signals::Signal;
      use crate::signals::SignalArbiter;

      let arbiter = SignalArbiter::emit_app(
        Signal::with_capacity(super::signal_ids::QUEUE_JOB_QUEUED, 2)
          .meta("name", job_name)
          .meta("id", id.to_string()),
      );
      #[cfg(not(feature = "compio"))]
      tokio::spawn(arbiter);
      #[cfg(feature = "compio")]
      compio::runtime::spawn(arbiter).detach();
    }
    Ok(id)
  }
}

fn serialize(
  payload: &(impl serde::Serialize + ?Sized),
  limit: Option<usize>,
) -> Result<Vec<u8>, EnqueueError> {
  let Some(limit) = limit else {
    return serde_json::to_vec(payload)
      .map_err(|error| QueueError::SerializeError(error.to_string()).into());
  };
  let mut writer = LimitedWriter {
    bytes: Vec::new(),
    limit,
    exceeded: false,
  };
  match serde_json::to_writer(&mut writer, payload) {
    Ok(()) => Ok(writer.bytes),
    Err(_) if writer.exceeded => Err(CapacityExceeded::PayloadBytes { limit }.into()),
    Err(error) => Err(QueueError::SerializeError(error.to_string()).into()),
  }
}

struct LimitedWriter {
  bytes: Vec<u8>,
  limit: usize,
  exceeded: bool,
}

impl io::Write for LimitedWriter {
  fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
    if bytes.len() > self.limit - self.bytes.len() {
      self.exceeded = true;
      return Err(io::Error::other("payload exceeds queue capacity"));
    }
    let needed = self.bytes.len() + bytes.len();
    if needed > self.bytes.capacity() {
      let capacity = needed
        .max(self.bytes.capacity().saturating_mul(2))
        .min(self.limit);
      self.bytes.reserve_exact(capacity - self.bytes.len());
    }
    self.bytes.extend_from_slice(bytes);
    Ok(bytes.len())
  }

  fn flush(&mut self) -> io::Result<()> {
    Ok(())
  }
}
