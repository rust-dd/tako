//! Per-router typed state and signal-arbiter accessors.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::Router;
use crate::router_state::RouterState;
#[cfg(feature = "signals")]
use crate::signals::Signal;
#[cfg(feature = "signals")]
use crate::signals::SignalArbiter;

impl Router {
  /// Stores `value` as this router's `T`, replacing an earlier one; `State<T>` reads it
  /// before falling back to the process-global [`get_state`](crate::state::get_state).
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::router::Router;
  ///
  /// #[derive(Clone)]
  /// struct Db;
  ///
  /// let mut router = Router::new();
  /// router.with_state(Db);
  /// ```
  pub fn with_state<T: Clone + Send + Sync + 'static>(&mut self, value: T) -> &mut Self {
    self.router_state.insert(value);
    self.has_router_state.store(true, Ordering::Release);
    self
  }

  /// The state that [`Router::with_state`] writes to.
  #[inline]
  pub fn router_state(&self) -> &Arc<RouterState> {
    &self.router_state
  }

  #[cfg(feature = "signals")]
  /// This router's arbiter; request start and completion signals are emitted on it.
  pub fn signals(&self) -> &SignalArbiter {
    &self.signals
  }

  #[cfg(feature = "signals")]
  /// Returns a clone of the signal arbiter, useful for sharing through state.
  pub fn signal_arbiter(&self) -> SignalArbiter {
    self.signals.clone()
  }

  #[cfg(feature = "signals")]
  /// Registers a handler for a named signal on this router's arbiter.
  pub fn on_signal<F, Fut>(&self, id: impl Into<String>, handler: F)
  where
    F: Fn(Signal) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
  {
    self.signals.on(id, handler);
  }

  #[cfg(feature = "signals")]
  /// Emits a signal through this router's arbiter.
  pub async fn emit_signal(&self, signal: Signal) {
    self.signals.emit(signal).await;
  }
}
