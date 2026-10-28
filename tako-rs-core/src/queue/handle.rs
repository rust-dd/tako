use std::sync::Arc;
use std::time::Duration;

use super::runtime::QueueInner;

/// A queue worker runtime failed or exceeded its shutdown deadline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum QueueRunError {
  /// The shutdown deadline expired; remaining workers have been asked to cancel.
  Timeout,
  /// A worker panicked outside the job handler's panic boundary.
  WorkerPanicked,
  /// The runtime or its last owner dropped a worker before completion.
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

/// Shared ownership of a queue's workers on either runtime.
///
/// The last handle's drop cancels workers, including when handlers retain queue clones.
/// Keep a handle until shutdown completes; cloning it does not create more workers.
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

  /// Waits until every worker has exited and released its running job, without stopping it.
  pub async fn join(&self) {
    self.owner.0.join_workers().await;
  }

  /// Waits for every worker and returns the shared runtime result to every caller.
  pub async fn result(&self) -> Result<(), QueueRunError> {
    self.owner.0.worker_result().await
  }

  /// Stops the queue and waits at most `timeout`, reporting deadline expiry distinctly.
  ///
  /// On timeout, cancellation is requested and this method returns immediately. Use
  /// [`join`](Self::join) to await resource release. Handlers must yield to the runtime
  /// for cancellation to run; synchronous blocking code cannot be interrupted.
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
