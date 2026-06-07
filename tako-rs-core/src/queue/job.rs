//! Job and dead-letter task types passed across the queue.

use std::time::Instant;

use super::QueueError;

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
