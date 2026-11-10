use std::io;

use crate::PerThreadError;
use crate::PerThreadErrorKind;
use crate::PerThreadReadiness;

/// Sticky shutdown signal: workers that start waiting later still receive it.
#[derive(Clone)]
pub struct PerThreadShutdown {
  pub(crate) inner: tokio_util::sync::CancellationToken,
  pub(crate) readiness: PerThreadReadiness,
}

impl PerThreadShutdown {
  pub(crate) fn new(workers: usize) -> Self {
    Self {
      inner: tokio_util::sync::CancellationToken::new(),
      readiness: PerThreadReadiness::new(workers),
    }
  }

  /// Clears readiness and starts draining all workers.
  pub fn trigger(&self) {
    self.readiness.stop();
    self.inner.cancel();
  }

  /// Resolves once shutdown has been triggered.
  pub async fn notified(&self) {
    self.inner.cancelled().await;
  }

  pub(crate) fn report_bind_success(&self, worker_id: usize) {
    self.readiness.listening(worker_id);
  }

  #[cfg(not(feature = "compio"))]
  pub(crate) fn report_handoff_only(&self, worker_id: usize, error: io::Error) {
    if self.readiness.handoff_only(worker_id, error) {
      self.trigger();
    }
  }

  pub(crate) fn report_exit(&self, worker_id: usize, outcome: Result<(), PerThreadError>) {
    if self.readiness.finished(worker_id, outcome) {
      self.trigger();
    }
  }

  /// Publishes failure before the caller drops its runtime and waits for blocking tasks.
  /// Final thread completion is accounted for separately by `report_exit`.
  pub(crate) fn supervise_runtime(
    &self,
    worker_id: usize,
    run: impl FnOnce() -> Result<(), PerThreadError>,
  ) -> Result<(), PerThreadError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
      Ok(result) => {
        let result = if result.is_ok() && !self.inner.is_cancelled() {
          Err(PerThreadError::new(
            worker_id,
            PerThreadErrorKind::UnexpectedExit,
            io::Error::other("worker exited without a shutdown request"),
          ))
        } else {
          result
        };

        if let Err(error) = &result
          && self.readiness.failed(worker_id, error.clone())
        {
          self.trigger();
        }

        result
      }
      Err(payload) => {
        self.readiness.failed(
          worker_id,
          PerThreadError::panic(worker_id, payload.as_ref()),
        );
        self.trigger();
        std::panic::resume_unwind(payload);
      }
    }
  }

  /// Waits for readiness, failure, or shutdown. `total` is ignored; spawning sets the count.
  pub async fn wait_for_bind_outcome(&self, _total: usize) -> io::Result<()> {
    self.readiness.wait_ready().await
  }
}
