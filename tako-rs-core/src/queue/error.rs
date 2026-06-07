//! Error type for queue operations.

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
