//! In-process signal arbiter and dispatch system.
//!
//! This module defines a small abstraction for named signals that can be emitted
//! and handled within a Tako application. It is intended for cross-cutting
//! concerns such as metrics, logging hooks, or custom application events.

use std::{
  collections::HashMap,
  sync::Arc,
};

use dashmap::DashMap;
use futures_util::future::{join_all, BoxFuture};
use once_cell::sync::Lazy;

/// Well-known signal identifiers for common lifecycle and request events.
pub mod ids {
  pub const SERVER_STARTED: &str = "server.started";
  pub const SERVER_STOPPED: &str = "server.stopped";
  pub const CONNECTION_OPENED: &str = "connection.opened";
  pub const CONNECTION_CLOSED: &str = "connection.closed";
  pub const REQUEST_STARTED: &str = "request.started";
  pub const REQUEST_COMPLETED: &str = "request.completed";
  pub const ROUTER_HOT_RELOAD: &str = "router.hot_reload";
}

/// A signal emitted through the arbiter.
///
/// Signals are identified by an arbitrary string and can carry a map of
/// metadata. Callers are free to define their own conventions for ids and
/// fields.
#[derive(Clone, Debug, Default)]
pub struct Signal {
  /// Identifier of the signal, for example "request.started" or "metrics.tick".
  pub id: String,
  /// Optional metadata payload carried with the signal.
  pub metadata: HashMap<String, String>,
}

impl Signal {
  /// Creates a new signal with the given id and empty metadata.
  pub fn new(id: impl Into<String>) -> Self {
    Self {
      id: id.into(),
      metadata: HashMap::new(),
    }
  }

  /// Creates a new signal with initial metadata.
  pub fn with_metadata(id: impl Into<String>, metadata: HashMap<String, String>) -> Self {
    Self {
      id: id.into(),
      metadata,
    }
  }
}

/// Boxed async signal handler.
pub type SignalHandler = Arc<dyn Fn(Signal) -> BoxFuture<'static, ()> + Send + Sync>;

#[derive(Default)]
struct Inner {
  handlers: DashMap<String, Vec<SignalHandler>>,
}

/// Shared arbiter used to register and dispatch named signals.
#[derive(Clone, Default)]
pub struct SignalArbiter {
  inner: Arc<Inner>,
}

/// Global application-level signal arbiter.
static APP_SIGNAL_ARBITER: Lazy<SignalArbiter> = Lazy::new(SignalArbiter::new);

/// Returns a reference to the global application-level signal arbiter.
pub fn app_signals() -> &'static SignalArbiter {
  &APP_SIGNAL_ARBITER
}

/// Alias for using the signal arbiter as a general event bus.
pub type EventBus = SignalArbiter;

/// Returns the global application-level event bus.
pub fn app_events() -> &'static EventBus {
  app_signals()
}

impl SignalArbiter {
  /// Creates a new, empty signal arbiter.
  pub fn new() -> Self {
    Self::default()
  }

  /// Registers a handler for the given signal id.
  ///
  /// Handlers are invoked in registration order whenever a matching signal is emitted.
  pub fn on<F, Fut>(&self, id: impl Into<String>, handler: F)
  where
    F: Fn(Signal) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
  {
    let id = id.into();
    let handler: SignalHandler = Arc::new(move |signal: Signal| {
      let fut = handler(signal);
      Box::pin(async move { fut.await })
    });

    self
      .inner
      .handlers
      .entry(id)
      .or_insert_with(Vec::new)
      .push(handler);
  }

  /// Emits a signal and awaits all registered handlers.
  ///
  /// Handlers run concurrently and this method resolves once all handlers have completed.
  pub async fn emit(&self, signal: Signal) {
    if let Some(entry) = self.inner.handlers.get(&signal.id) {
      let handlers = entry.clone();
      drop(entry);

      let futures = handlers.into_iter().map(|handler| {
        let s = signal.clone();
        handler(s)
      });

      let _ = join_all(futures).await;
    }
  }

  /// Emits a signal using the global application-level arbiter.
  pub async fn emit_app(signal: Signal) {
    app_signals().emit(signal).await;
  }

  /// Merges all handlers from `other` into `self`.
  ///
  /// This is used by router merging so that signal handlers attached to
  /// a merged router continue to be active.
  pub fn merge_from(&self, other: &SignalArbiter) {
    for entry in other.inner.handlers.iter() {
      let id = entry.key().clone();
      let handlers = entry.value().clone();

      self
        .inner
        .handlers
        .entry(id)
        .or_insert_with(Vec::new)
        .extend(handlers);
    }
  }
}
