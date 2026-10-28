use std::sync::Arc;
use std::time::Duration;

use super::runtime::QueueInner;

/// Queue worker failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum QueueRunError {
  /// Shutdown deadline expired; cancellation was requested.
  Timeout,
  /// Worker panic outside a job handler.
  WorkerPanicked,
  /// Worker cancelled before completion.
  Aborted,
}

impl std::fmt::Display for QueueRunError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(match self {
      Self::Timeout => "queue shutdown timed out",
      Self::WorkerPanicked => "queue worker panicked",
      Self::Aborted => "queue workers were aborted",
    })
  }
}

impl std::error::Error for QueueRunError {}

/// Worker lifecycle handle; dropping the last clone cancels its workers.
#[derive(Clone)]
pub struct QueueHandle {
  pub(crate) owner: Arc<HandleOwner>,
}

pub(crate) struct HandleOwner(pub(crate) Arc<QueueInner>);

impl QueueHandle {
  /// Stops accepting jobs and moves unstarted jobs to dead letters without waiting.
  pub fn trigger(&self) {
    self.owner.0.trigger();
  }

  /// Waits for every worker to exit without triggering shutdown.
  pub async fn join(&self) {
    self.owner.0.join_workers().await;
  }

  /// Waits for workers and returns their shared completion result.
  pub async fn result(&self) -> Result<(), QueueRunError> {
    self.owner.0.worker_result().await
  }

  /// Stops the queue, cancelling workers if `timeout` expires.
  ///
  /// After a timeout, [`join`](Self::join) waits for cooperative cancellation to finish.
  pub async fn shutdown(&self, timeout: Duration) -> Result<(), QueueRunError> {
    self.owner.0.shutdown_workers(timeout).await
  }
}

impl Drop for HandleOwner {
  fn drop(&mut self) {
    self.0.abort_workers(QueueRunError::Aborted);
  }
}

impl std::fmt::Debug for QueueHandle {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("QueueHandle").finish_non_exhaustive()
  }
}
