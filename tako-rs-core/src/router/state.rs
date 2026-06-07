//! Per-router typed state and signal-arbiter accessors.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::Router;
use crate::router_state::RouterState;
#[cfg(feature = "signals")]
use crate::signals::Signal;
#[cfg(feature = "signals")]
use crate::signals::SignalArbiter;
use crate::state::set_state;

impl Router {
  /// Adds a value to the global type-based state accessible by all handlers.
  ///
  /// Global state allows sharing data across different routes and middleware.
  /// Values are stored by their concrete type and retrieved via the
  /// `State` extractor (from `tako-extractors`) or with
  /// [`crate::state::get_state`].
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::router::Router;
  ///
  /// #[derive(Clone)]
  /// struct AppConfig { database_url: String, api_key: String }
  ///
  /// let mut router = Router::new();
  /// router.state(AppConfig {
  ///     database_url: "postgresql://localhost/mydb".to_string(),
  ///     api_key: "secret-key".to_string(),
  /// });
  /// // You can also store simple types by type:
  /// router.state::<String>("1.0.0".to_string());
  /// ```
  pub fn state<T: Clone + Send + Sync + 'static>(&mut self, value: T) {
    set_state(value);
  }

  /// Inserts a value into this router's instance-local typed state.
  ///
  /// Unlike [`Router::state`] (which writes the process-global registry and
  /// therefore allows only one value per `T` per process), `with_state` is
  /// per-router — multiple routers can hold distinct `T`s without collisions.
  ///
  /// The `State` extractor (from `tako-extractors`) reads the per-router
  /// store first and falls back to the global store if no per-router value
  /// exists, so existing code that uses `set_state` / `Router::state`
  /// continues to work unchanged.
  ///
  /// Hot-path cost is one `Arc` clone per request *only when* at least one
  /// `with_state` call has happened on this router; an `AtomicBool::Acquire`
  /// fast-path skips it for routers that don't use instance-local state.
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

  /// Returns the per-router typed state (shared `Arc`).
  #[inline]
  pub fn router_state(&self) -> &Arc<RouterState> {
    &self.router_state
  }

  #[cfg(feature = "signals")]
  /// Returns a reference to the signal arbiter.
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
