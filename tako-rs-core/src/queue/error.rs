/// Error from enqueueing a job or from running its handler.
#[derive(Debug)]
pub enum QueueError {
  SerializeError(String),
  /// A capacity limit was reached, a handler failed, or a payload did not deserialize.
  HandlerError(String),
  /// The queue is shutting down and accepts no more jobs.
  Shutdown,
}

impl std::fmt::Display for QueueError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::SerializeError(e) => write!(f, "failed to serialize job payload: {e}"),
      Self::HandlerError(e) => write!(f, "job handler error: {e}"),
      Self::Shutdown => write!(f, "queue has been shut down"),
    }
  }
}

impl std::error::Error for QueueError {}
