use std::io;

use crate::PerThreadError;
use crate::PerThreadReadiness;
use crate::PerThreadShutdown;

/// Owns worker threads; dropping it requests shutdown without blocking.
pub struct PerThreadHandle {
  workers: Vec<std::thread::JoinHandle<()>>,
  shutdown: PerThreadShutdown,
}

impl PerThreadHandle {
  pub(crate) fn new(
    workers: Vec<std::thread::JoinHandle<()>>,
    shutdown: PerThreadShutdown,
  ) -> Self {
    Self { workers, shutdown }
  }

  /// Remains usable after the server handle is dropped.
  pub fn readiness(&self) -> PerThreadReadiness {
    self.shutdown.readiness.clone()
  }

  pub fn shutdown_handle(&self) -> PerThreadShutdown {
    self.shutdown.clone()
  }

  /// Stops accepting and starts the configured drain period.
  pub fn shutdown(&self) {
    self.shutdown.trigger();
  }

  pub async fn wait_ready(&self) -> io::Result<()> {
    self.shutdown.readiness.wait_ready().await
  }

  /// Resolves at the first terminal failure, or after every worker stops cleanly.
  /// Use [`Self::join`] to wait for peer cleanup after a failure.
  pub async fn result(&self) -> Result<(), PerThreadError> {
    self.shutdown.readiness.result().await
  }

  /// Blocks until worker threads exit and returns the retained terminal result.
  /// Call `shutdown()` first when the server is still running.
  pub fn join(mut self) -> Result<(), PerThreadError> {
    let mut join_error = None;
    for (worker_id, worker) in std::mem::take(&mut self.workers).into_iter().enumerate() {
      if let Err(payload) = worker.join() {
        join_error.get_or_insert_with(|| PerThreadError::panic(worker_id, payload.as_ref()));
      }
    }

    self
      .shutdown
      .readiness
      .failure()
      .or(join_error)
      .map_or(Ok(()), Err)
  }
}

impl Drop for PerThreadHandle {
  fn drop(&mut self) {
    self.shutdown.trigger();
  }
}
