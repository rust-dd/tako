//! In-process signal arbiter and dispatch system.
//!
//! This module defines a small abstraction for named signals that can be emitted
//! and handled within a Tako application. It is intended for cross-cutting
//! concerns such as metrics, logging hooks, or custom application events.

use std::{any::Any, collections::HashMap, sync::Arc};

use dashmap::DashMap;
use futures_util::future::{BoxFuture, join_all};
use once_cell::sync::Lazy;
use tokio::sync::broadcast;

const DEFAULT_BROADCAST_CAPACITY: usize = 64;

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

/// Boxed typed RPC handler used by the event bus.
pub type RpcHandler = Arc<
  dyn Fn(Arc<dyn Any + Send + Sync>) -> BoxFuture<'static, Arc<dyn Any + Send + Sync>>
    + Send
    + Sync,
>;

#[derive(Default)]
struct Inner {
  handlers: DashMap<String, Vec<SignalHandler>>,
  topics: DashMap<String, broadcast::Sender<Signal>>,
  rpc: DashMap<String, RpcHandler>,
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

  /// Returns (and lazily initializes) the broadcast sender for a signal id.
  fn topic_sender(&self, id: &str) -> broadcast::Sender<Signal> {
    if let Some(existing) = self.inner.topics.get(id) {
      existing.clone()
    } else {
      let (tx, _rx) = broadcast::channel(DEFAULT_BROADCAST_CAPACITY);
      let entry = self.inner.topics.entry(id.to_string()).or_insert(tx);
      entry.clone()
    }
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

  /// Subscribes to a broadcast channel for the given signal id.
  ///
  /// This is useful for long-lived listeners such as metrics collectors,
  /// background workers, plugins, or middleware driven tasks.
  pub fn subscribe(&self, id: impl AsRef<str>) -> broadcast::Receiver<Signal> {
    let id_str = id.as_ref();
    let sender = self.topic_sender(id_str);
    sender.subscribe()
  }

  /// Broadcasts a signal to all subscribers without awaiting handler completion.
  pub fn broadcast(&self, signal: Signal) {
    if let Some(sender) = self.inner.topics.get(&signal.id) {
      let _ = sender.send(signal);
    }
  }

  /// Waits for the next occurrence of a signal id (oneshot-style).
  ///
  /// This uses the broadcast channel under the hood but resolves on the
  /// first successfully received signal.
  pub async fn once(&self, id: impl AsRef<str>) -> Option<Signal> {
    let mut rx = self.subscribe(id);
    loop {
      match rx.recv().await {
        Ok(sig) => return Some(sig),
        Err(broadcast::error::RecvError::Lagged(_)) => continue,
        Err(_) => return None,
      }
    }
  }

  /// Registers a typed RPC handler under the given id.
  ///
  /// This allows request/response style interactions over the same arbiter,
  /// using type-erased storage internally for flexibility.
  pub fn register_rpc<Req, Res, F, Fut>(&self, id: impl Into<String>, f: F)
  where
    Req: Send + Sync + 'static,
    Res: Send + Sync + 'static,
    F: Fn(Arc<Req>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Res> + Send + 'static,
  {
    let id_str = id.into();
    let id_for_panic = id_str.clone();
    let func = Arc::new(f);

    let handler: RpcHandler = Arc::new(move |raw: Arc<dyn Any + Send + Sync>| {
      let func = func.clone();
      let id_for_panic = id_for_panic.clone();
      Box::pin(async move {
        let req = raw
          .downcast::<Req>()
          .unwrap_or_else(|_| panic!("Signal RPC type mismatch for id: {}", id_for_panic));
        let res = func(req).await;
        Arc::new(res) as Arc<dyn Any + Send + Sync>
      })
    });

    self.inner.rpc.insert(id_str, handler);
  }

  /// Calls a typed RPC handler and returns a shared pointer to the response.
  pub async fn call_rpc_arc<Req, Res>(&self, id: impl AsRef<str>, req: Req) -> Option<Arc<Res>>
  where
    Req: Send + Sync + 'static,
    Res: Send + Sync + 'static,
  {
    let id_str = id.as_ref();
    let entry = self.inner.rpc.get(id_str)?;
    let handler = entry.clone();
    drop(entry);

    let raw_req: Arc<dyn Any + Send + Sync> = Arc::new(req);
    let raw_res = handler(raw_req).await;

    match raw_res.downcast::<Res>() {
      Ok(res) => Some(res),
      Err(_) => None,
    }
  }

  /// Calls a typed RPC handler and returns an owned response.
  pub async fn call_rpc<Req, Res>(&self, id: impl AsRef<str>, req: Req) -> Option<Res>
  where
    Req: Send + Sync + 'static,
    Res: Send + Sync + Clone + 'static,
  {
    self
      .call_rpc_arc::<Req, Res>(id, req)
      .await
      .map(|arc| (*arc).clone())
  }

  /// Emits a signal and awaits all registered handlers.
  ///
  /// Handlers run concurrently and this method resolves once all handlers have completed.
  pub async fn emit(&self, signal: Signal) {
    // First, broadcast to any subscribers.
    self.broadcast(signal.clone());

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

    for entry in other.inner.topics.iter() {
      let id = entry.key().clone();
      let sender = entry.value().clone();
      self.inner.topics.entry(id).or_insert(sender);
    }

    for entry in other.inner.rpc.iter() {
      let id = entry.key().clone();
      let handler = entry.value().clone();
      self.inner.rpc.insert(id, handler);
    }
  }
}
