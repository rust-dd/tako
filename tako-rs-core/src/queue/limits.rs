use std::fmt;

use super::QueueError;

/// All limits default to unbounded.
#[derive(Debug, Clone, Copy, Default)]
pub struct QueueLimits {
  pub(crate) jobs: Option<usize>,
  pub(crate) payload_bytes: Option<usize>,
  pub(crate) dead_letters: Option<usize>,
  pub(crate) dead_letter_bytes: Option<usize>,
}

impl QueueLimits {
  pub fn new() -> Self {
    Self::default()
  }

  /// Caps ready, delayed, running and retrying jobs together. Zero rejects new jobs.
  pub fn max_jobs(mut self, max: usize) -> Self {
    self.jobs = Some(max);
    self
  }

  /// Caps serialized payload bytes, counted once per outstanding job.
  pub fn max_payload_bytes(mut self, max: usize) -> Self {
    self.payload_bytes = Some(max);
    self
  }

  /// Keeps the newest `max` dead letters; zero disables their retention.
  pub fn max_dead_letters(mut self, max: usize) -> Self {
    self.dead_letters = Some(max);
    self
  }

  /// Caps retained dead-letter payload bytes, evicting oldest entries first.
  pub fn max_dead_letter_bytes(mut self, max: usize) -> Self {
    self.dead_letter_bytes = Some(max);
    self
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CapacityExceeded {
  Jobs { limit: usize },
  PayloadBytes { limit: usize },
}

impl fmt::Display for CapacityExceeded {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Jobs { limit } => write!(f, "queue capacity exceeded: {limit} jobs"),
      Self::PayloadBytes { limit } => write!(f, "queue capacity exceeded: {limit} payload bytes"),
    }
  }
}

impl std::error::Error for CapacityExceeded {}

#[derive(Debug)]
#[non_exhaustive]
pub enum EnqueueError {
  Queue(QueueError),
  Capacity(CapacityExceeded),
}

impl fmt::Display for EnqueueError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Queue(error) => error.fmt(f),
      Self::Capacity(error) => error.fmt(f),
    }
  }
}

impl std::error::Error for EnqueueError {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    Some(match self {
      Self::Queue(error) => error,
      Self::Capacity(error) => error,
    })
  }
}

impl From<QueueError> for EnqueueError {
  fn from(error: QueueError) -> Self {
    Self::Queue(error)
  }
}

impl From<CapacityExceeded> for EnqueueError {
  fn from(error: CapacityExceeded) -> Self {
    Self::Capacity(error)
  }
}

impl From<EnqueueError> for QueueError {
  fn from(error: EnqueueError) -> Self {
    match error {
      EnqueueError::Queue(error) => error,
      EnqueueError::Capacity(error) => Self::HandlerError(error.to_string()),
    }
  }
}
