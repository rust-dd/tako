use std::sync::Arc;

use futures_util::future::AbortHandle;
use parking_lot::Mutex;
use tokio::sync::Notify;

#[cfg(all(test, feature = "ws"))]
mod tests;

/// Owns upgraded sessions for one server or worker, including after HTTP hands off the IO.
#[derive(Default)]
pub struct UpgradeSupervisor {
  registry: UpgradeRegistry,
}

/// Registers upgraded sessions with their server's shutdown lifecycle.
#[derive(Clone, Default)]
pub struct UpgradeRegistry(Arc<Inner>);

#[derive(Default)]
struct Inner {
  state: Mutex<State>,
  stopping: Notify,
  drained: Notify,
}

#[derive(Default)]
struct State {
  stopping: bool,
  tasks: Vec<Option<AbortHandle>>,
  #[cfg(feature = "ws")]
  free: Vec<usize>,
  active: usize,
}

impl UpgradeSupervisor {
  /// Shares a registry with an outer task that also awaits cancelled sessions.
  pub fn new(registry: UpgradeRegistry) -> Self {
    Self { registry }
  }

  pub fn registry(&self) -> UpgradeRegistry {
    self.registry.clone()
  }

  /// Rejects new sessions and asks existing sessions to close gracefully.
  pub fn trigger(&self) {
    self.registry.0.state.lock().stopping = true;
    self.registry.0.stopping.notify_waiters();
  }

  /// Cancels remaining sessions on their next poll, releasing their underlying sockets.
  pub fn abort(&self) {
    self.trigger();
    let tasks = self
      .registry
      .0
      .state
      .lock()
      .tasks
      .iter()
      .flatten()
      .cloned()
      .collect::<Vec<_>>();
    for task in tasks {
      task.abort();
    }
  }

  /// Waits until all registered session futures have been dropped.
  pub async fn join(&self) {
    self.registry.join().await;
  }
}

impl Drop for UpgradeSupervisor {
  fn drop(&mut self) {
    self.abort();
  }
}

impl UpgradeRegistry {
  /// Waits until all registered session futures have been dropped.
  pub async fn join(&self) {
    loop {
      let notified = self.0.drained.notified();
      tokio::pin!(notified);
      notified.as_mut().enable();
      if self.0.state.lock().active == 0 {
        return;
      }
      notified.await;
    }
  }
  /// Adds lifecycle metadata only when Hyper offers an HTTP upgrade.
  pub fn attach<B>(
    &self,
    #[cfg_attr(not(feature = "ws"), allow(unused_variables))] request: &mut http::Request<B>,
  ) {
    #[cfg(feature = "ws")]
    if request
      .extensions()
      .get::<hyper::upgrade::OnUpgrade>()
      .is_some()
    {
      request.extensions_mut().insert(self.clone());
    }
  }

  #[cfg(feature = "ws")]
  pub(crate) fn register(&self) -> Option<UpgradeSession> {
    let mut state = self.0.state.lock();
    if state.stopping {
      return None;
    }
    let (handle, registration) = AbortHandle::new_pair();
    let slot = if let Some(slot) = state.free.pop() {
      state.tasks[slot] = Some(handle);
      slot
    } else {
      state.tasks.push(Some(handle));
      state.tasks.len() - 1
    };
    state.active += 1;
    Some(UpgradeSession {
      registry: self.clone(),
      slot,
      registration: Some(registration),
    })
  }

  #[cfg(feature = "ws")]
  pub(crate) async fn stopping(&self) {
    loop {
      let notified = self.0.stopping.notified();
      tokio::pin!(notified);
      notified.as_mut().enable();
      if self.0.state.lock().stopping {
        return;
      }
      notified.await;
    }
  }
}

#[cfg(feature = "ws")]
pub(crate) struct UpgradeSession {
  registry: UpgradeRegistry,
  slot: usize,
  registration: Option<futures_util::future::AbortRegistration>,
}

#[cfg(feature = "ws")]
impl UpgradeSession {
  pub(crate) fn registry(&self) -> UpgradeRegistry {
    self.registry.clone()
  }

  pub(crate) async fn run(mut self, future: impl std::future::Future<Output = ()>) {
    let registration = self.registration.take().expect("session runs once");
    let _ = futures_util::future::Abortable::new(future, registration).await;
  }
}

#[cfg(feature = "ws")]
impl Drop for UpgradeSession {
  fn drop(&mut self) {
    let mut state = self.registry.0.state.lock();
    state.tasks[self.slot] = None;
    state.free.push(self.slot);
    state.active -= 1;
    if state.active == 0 {
      self.registry.0.drained.notify_waiters();
    }
  }
}
