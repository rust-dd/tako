use std::io;
use std::sync::Arc;
use std::sync::Mutex;

use crate::PerThreadError;
use crate::PerThreadErrorKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PerThreadWorkerState {
  Starting,
  Listening,
  /// Receives handoffs from other Tokio workers after its own bind failed.
  HandoffOnly,
  Stopped,
  /// Failed, but runtime cleanup may still be running.
  Failed,
}

struct State {
  workers: Vec<PerThreadWorkerState>,
  remaining: usize,
  stopping: bool,
  failure: Option<PerThreadError>,
  bind_failure: Option<PerThreadError>,
}

impl State {
  fn record_failure(&mut self, worker_id: usize, error: PerThreadError) {
    self.workers[worker_id] = PerThreadWorkerState::Failed;
    if error.kind() == PerThreadErrorKind::Bind {
      self.bind_failure.get_or_insert(error);
    } else {
      self.failure.get_or_insert(error);
    }

    self.check_bind_failure();
  }

  fn is_ready(&self) -> bool {
    !self.stopping
      && self.failure.is_none()
      && self.workers.contains(&PerThreadWorkerState::Listening)
  }

  fn check_bind_failure(&mut self) {
    if !self.workers.iter().any(|state| {
      matches!(
        state,
        PerThreadWorkerState::Starting | PerThreadWorkerState::Listening
      )
    }) && self.failure.is_none()
      && !self.stopping
    {
      self.failure = self.bind_failure.clone();
    }
  }
}

struct Shared {
  state: Mutex<State>,
  notify: tokio::sync::Notify,
}

/// Shared readiness for health probes. Requires a listener and clears on shutdown or
/// terminal failure; a bind failure is tolerated while another worker listens.
#[derive(Clone)]
pub struct PerThreadReadiness {
  shared: Arc<Shared>,
}

impl PerThreadReadiness {
  pub(crate) fn new(workers: usize) -> Self {
    Self {
      shared: Arc::new(Shared {
        state: Mutex::new(State {
          workers: vec![PerThreadWorkerState::Starting; workers],
          remaining: workers,
          stopping: false,
          failure: None,
          bind_failure: None,
        }),
        notify: tokio::sync::Notify::new(),
      }),
    }
  }

  fn state(&self) -> std::sync::MutexGuard<'_, State> {
    self
      .shared
      .state
      .lock()
      .unwrap_or_else(std::sync::PoisonError::into_inner)
  }

  pub fn is_ready(&self) -> bool {
    self.state().is_ready()
  }

  /// States indexed by worker ID.
  pub fn workers(&self) -> Vec<PerThreadWorkerState> {
    self.state().workers.clone()
  }

  /// The first terminal failure, retained for all observers.
  pub fn failure(&self) -> Option<PerThreadError> {
    self.state().failure.clone()
  }

  /// Waits for a listener, failing on worker failure or shutdown before readiness.
  pub async fn wait_ready(&self) -> io::Result<()> {
    loop {
      let notified = self.shared.notify.notified();
      tokio::pin!(notified);
      notified.as_mut().enable();

      {
        let state = self.state();
        if let Some(error) = &state.failure {
          return Err(error.clone().into());
        }

        if state.is_ready() {
          return Ok(());
        }

        if state.stopping || state.remaining == 0 {
          return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "per-thread server stopped",
          ));
        }
      }

      notified.await;
    }
  }

  pub(crate) async fn result(&self) -> Result<(), PerThreadError> {
    loop {
      let notified = self.shared.notify.notified();
      tokio::pin!(notified);
      notified.as_mut().enable();

      {
        let state = self.state();
        if let Some(error) = &state.failure {
          return Err(error.clone());
        }

        if state.remaining == 0 {
          return Ok(());
        }
      }

      notified.await;
    }
  }

  pub(crate) fn stop(&self) {
    self.state().stopping = true;
    self.shared.notify.notify_waiters();
  }

  pub(crate) fn listening(&self, worker_id: usize) {
    self.state().workers[worker_id] = PerThreadWorkerState::Listening;
    self.shared.notify.notify_waiters();
  }

  #[cfg(not(feature = "compio"))]
  pub(crate) fn handoff_only(&self, worker_id: usize, error: io::Error) -> bool {
    let failed = {
      let mut state = self.state();
      state.workers[worker_id] = PerThreadWorkerState::HandoffOnly;
      state
        .bind_failure
        .get_or_insert_with(|| PerThreadError::new(worker_id, PerThreadErrorKind::Bind, error));
      state.check_bind_failure();
      state.failure.is_some()
    };

    self.shared.notify.notify_waiters();
    failed
  }

  pub(crate) fn finished(&self, worker_id: usize, outcome: Result<(), PerThreadError>) -> bool {
    let failed = {
      let mut state = self.state();
      let outcome = if outcome.is_ok() && !state.stopping {
        Err(PerThreadError::new(
          worker_id,
          PerThreadErrorKind::UnexpectedExit,
          io::Error::other("worker exited without a shutdown request"),
        ))
      } else {
        outcome
      };

      state.remaining -= 1;
      match outcome {
        Ok(()) => state.workers[worker_id] = PerThreadWorkerState::Stopped,
        Err(error) => state.record_failure(worker_id, error),
      }
      state.check_bind_failure();
      state.failure.is_some()
    };

    self.shared.notify.notify_waiters();
    failed
  }

  pub(crate) fn failed(&self, worker_id: usize, error: PerThreadError) -> bool {
    let failed = {
      let mut state = self.state();
      state.record_failure(worker_id, error);
      state.failure.is_some()
    };

    self.shared.notify.notify_waiters();
    failed
  }
}
