use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::Weak;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use futures_util::FutureExt;
use futures_util::future::AbortHandle;
use futures_util::future::Abortable;
use futures_util::future::Either;
use parking_lot::Mutex;
use tokio::sync::Notify;

use super::DeadJob;
use super::QueueHandle;
use super::QueueRunError;
use super::handle::HandleOwner;
use super::pending::PendingJob;
use super::runtime::QueueInner;
use super::worker::worker_loop;

#[derive(Default)]
pub(crate) struct Lifecycle {
  state: Mutex<WorkerState>,
  done: Notify,
}

#[derive(Default)]
struct WorkerState {
  started: bool,
  remaining: usize,
  done: bool,
  finishing: bool,
  error: Option<QueueRunError>,
  aborts: Vec<AbortHandle>,
  handle: Weak<HandleOwner>,
}

pub(crate) struct QueueOwner(pub(crate) Weak<QueueInner>);

impl Drop for QueueOwner {
  fn drop(&mut self) {
    if let Some(inner) = self.0.upgrade() {
      let unowned = inner.lifecycle.state.lock().handle.strong_count() == 0;
      if unowned {
        inner.abort_workers(QueueRunError::Aborted);
      }
    }
  }
}

impl QueueInner {
  pub(crate) fn handle(self: &Arc<Self>) -> QueueHandle {
    let mut state = self.lifecycle.state.lock();
    let owner = state.handle.upgrade().unwrap_or_else(|| {
      let owner = Arc::new(HandleOwner(self.clone()));
      state.handle = Arc::downgrade(&owner);
      owner
    });
    QueueHandle { owner }
  }

  pub(crate) fn start_workers(self: &Arc<Self>) {
    let tasks = {
      let mut state = self.lifecycle.state.lock();
      if state.started {
        return;
      }
      state.started = true;
      state.remaining = self.num_workers;
      (0..self.num_workers)
        .map(|_| {
          let (abort, registration) = AbortHandle::new_pair();
          state.aborts.push(abort);
          let completion = WorkerCompletion {
            inner: self.clone(),
            result: Err(QueueRunError::Aborted),
          };
          (completion, registration)
        })
        .collect::<Vec<_>>()
    };

    for (completion, registration) in tasks {
      let task = async move {
        let worker = worker_loop(completion.inner.clone());
        let result = match AssertUnwindSafe(Abortable::new(worker, registration))
          .catch_unwind()
          .await
        {
          Ok(Ok(())) => Ok(()),
          Ok(Err(_)) => Err(QueueRunError::Aborted),
          Err(_) => Err(QueueRunError::WorkerPanicked),
        };
        completion.finish(result);
      };
      #[cfg(not(feature = "compio"))]
      tokio::spawn(task);
      #[cfg(feature = "compio")]
      compio::runtime::spawn(task).detach();
    }
  }

  pub(crate) fn trigger(&self) {
    let idle = {
      let mut state = self.lifecycle.state.lock();
      let _pending = self.pending.lock();
      state.started = true;
      self.shutdown.store(true, Ordering::SeqCst);
      state.remaining == 0
    };
    self.drain_pending();
    self.notify.notify_waiters();
    if idle {
      self.finish_workers();
    }
  }

  pub(crate) fn abort_workers(&self, error: QueueRunError) {
    self.trigger();
    let aborts = {
      let mut state = self.lifecycle.state.lock();
      if state.done {
        return;
      }
      state.error.get_or_insert(error);
      state.aborts.clone()
    };
    for abort in aborts {
      abort.abort();
    }
  }

  pub(crate) async fn join_workers(&self) {
    loop {
      let mut notified = std::pin::pin!(self.lifecycle.done.notified());
      notified.as_mut().enable();
      if self.lifecycle.state.lock().done {
        return;
      }
      notified.await;
    }
  }

  pub(crate) async fn worker_result(&self) -> Result<(), QueueRunError> {
    self.join_workers().await;
    self.lifecycle.state.lock().error.map_or(Ok(()), Err)
  }

  pub(crate) async fn shutdown_workers(&self, timeout: Duration) -> Result<(), QueueRunError> {
    self.trigger();
    let completed = std::pin::pin!(self.worker_result());
    let deadline = std::pin::pin!(async {
      #[cfg(not(feature = "compio"))]
      tokio::time::sleep(timeout).await;
      #[cfg(feature = "compio")]
      compio::time::sleep(timeout).await;
    });
    match futures_util::future::select(completed, deadline).await {
      Either::Left((result, _)) => result,
      Either::Right(_) => {
        self.abort_workers(QueueRunError::Timeout);
        self.lifecycle.state.lock().error.map_or(Ok(()), Err)
      }
    }
  }

  pub(crate) fn dead_letter(&self, job: PendingJob, attempts: u32, error: String) {
    self.dead_letters.lock().push(Arc::new(DeadJob {
      id: job.id,
      name: job.name,
      payload: job.payload,
      attempts,
      error,
      failed_at: Instant::now(),
    }));
  }

  pub(crate) fn drain_pending(&self) {
    let mut pending = self.pending.lock();
    if pending.is_empty() {
      return;
    }
    for job in pending.drain() {
      let attempts = job.attempt;
      self.dead_letter(job, attempts, "queue shutdown before job ran".into());
    }
  }

  fn finish_workers(&self) {
    {
      let mut state = self.lifecycle.state.lock();
      if state.done || state.finishing {
        return;
      }
      state.finishing = true;
    }
    self.drain_pending();
    let handlers = std::mem::take(&mut *self.handlers.lock());
    for handler in handlers.into_values() {
      if std::panic::catch_unwind(AssertUnwindSafe(|| drop(handler))).is_err() {
        self
          .lifecycle
          .state
          .lock()
          .error
          .get_or_insert(QueueRunError::WorkerPanicked);
      }
    }
    {
      let mut state = self.lifecycle.state.lock();
      state.aborts.clear();
      state.done = true;
    }
    self.lifecycle.done.notify_waiters();
  }
}

struct WorkerCompletion {
  inner: Arc<QueueInner>,
  result: Result<(), QueueRunError>,
}

impl WorkerCompletion {
  fn finish(mut self, result: Result<(), QueueRunError>) {
    self.result = result;
    drop(self);
  }
}

impl Drop for WorkerCompletion {
  fn drop(&mut self) {
    if let Err(error) = self.result {
      self.inner.abort_workers(error);
    }
    let finished = {
      let mut state = self.inner.lifecycle.state.lock();
      state.remaining -= 1;
      state.remaining == 0
    };
    if finished {
      self.inner.finish_workers();
    }
  }
}
