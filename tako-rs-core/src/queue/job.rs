use std::time::Instant;

use super::QueueError;

/// A job as its handler receives it, with a JSON payload.
pub struct Job {
  pub(crate) payload: Vec<u8>,
  /// Name the handler was registered under.
  pub name: String,
  /// Zero for the first run.
  pub attempt: u32,
  pub id: u64,
}

impl Job {
  /// Fails with [`QueueError::HandlerError`], so `?` in a handler fails the job.
  pub fn deserialize<T: serde::de::DeserializeOwned>(&self) -> Result<T, QueueError> {
    serde_json::from_slice(&self.payload).map_err(|e| QueueError::HandlerError(e.to_string()))
  }

  /// The payload's JSON bytes.
  pub fn raw_payload(&self) -> &[u8] {
    &self.payload
  }
}

/// A job that exhausted its retries, had no handler, or never ran before shutdown.
#[derive(Debug, Clone)]
pub struct DeadJob {
  pub id: u64,
  pub name: String,
  pub payload: Vec<u8>,
  pub attempts: u32,
  pub error: String,
  /// When the job entered the dead letter queue.
  pub failed_at: Instant,
}
