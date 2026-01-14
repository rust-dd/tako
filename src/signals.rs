//! In-process signal arbiter and dispatch system.
//!
//! This module defines a small abstraction for named signals that can be emitted
//! and handled within a Tako application. It is intended for cross-cutting
//! concerns such as metrics, logging hooks, or custom application events.

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
#[cfg(not(feature = "compio"))]
use std::time::Duration;

use futures_util::future::BoxFuture;
use futures_util::future::join_all;
use once_cell::sync::Lazy;
use scc::HashMap as SccHashMap;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
#[cfg(not(feature = "compio"))]
use tokio::time::timeout;

use crate::types::BuildHasher;

const DEFAULT_BROADCAST_CAPACITY: usize = 64;
static GLOBAL_BROADCAST_CAPACITY: AtomicUsize = AtomicUsize::new(DEFAULT_BROADCAST_CAPACITY);

/// Well-known signal identifiers for common lifecycle and request events.
pub mod ids {
  pub const SERVER_STARTED: &str = "server.started";
  pub const SERVER_STOPPED: &str = "server.stopped";
  pub const CONNECTION_OPENED: &str = "connection.opened";
  pub const CONNECTION_CLOSED: &str = "connection.closed";
  pub const REQUEST_STARTED: &str = "request.started";
  pub const REQUEST_COMPLETED: &str = "request.completed";
  pub const ROUTER_HOT_RELOAD: &str = "router.hot_reload";
  pub const RPC_ERROR: &str = "rpc.error";
  pub const ROUTE_REQUEST_STARTED: &str = "route.request.started";
  pub const ROUTE_REQUEST_COMPLETED: &str = "route.request.completed";
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
  pub metadata: HashMap<String, String, BuildHasher>,
}

impl Signal {
  /// Creates a new signal with the given id and empty metadata.
  #[inline]
  #[must_use]
  pub fn new(id: impl Into<String>) -> Self {
    Self {
      id: id.into(),
      metadata: HashMap::with_hasher(BuildHasher::default()),
    }
  }

  /// Creates a new signal with initial metadata.
  #[inline]
  #[must_use]
  pub fn with_metadata(
    id: impl Into<String>,
    metadata: HashMap<String, String, BuildHasher>,
  ) -> Self {
    Self {
      id: id.into(),
      metadata,
    }
  }

  /// Creates a signal from a typed payload implementing `SignalPayload`.
  #[inline]
  #[must_use]
  pub fn from_payload<P: SignalPayload>(payload: &P) -> Self {
    Self {
      id: payload.id().to_string(),
      metadata: payload.to_metadata(),
    }
  }
}

/// Trait for types that can be converted into a `Signal`.
pub trait SignalPayload {
  /// The canonical id for this kind of signal, e.g. "request.completed".
  fn id(&self) -> &'static str;

  /// Serializes the payload into the metadata map.
  fn to_metadata(&self) -> HashMap<String, String, BuildHasher>;
}

/// Boxed async signal handler.
pub type SignalHandler = Arc<dyn Fn(Signal) -> BoxFuture<'static, ()> + Send + Sync>;

/// Boxed typed RPC handler used by the signal arbiter.
pub type RpcHandler = Arc<
  dyn Fn(Arc<dyn Any + Send + Sync>) -> BoxFuture<'static, Arc<dyn Any + Send + Sync>>
    + Send
    + Sync,
>;

/// Exporter callback invoked for every emitted signal.
pub type SignalExporter = Arc<dyn Fn(&Signal) + Send + Sync>;

/// Simple stream type returned by filtered subscriptions.
pub type SignalStream = mpsc::UnboundedReceiver<Signal>;

#[derive(Default)]
struct Inner {
  handlers: SccHashMap<String, Vec<SignalHandler>>,
  topics: SccHashMap<String, broadcast::Sender<Signal>>,
  rpc: SccHashMap<String, RpcHandler>,
  exporters: SccHashMap<u64, SignalExporter>,
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

/// Returns the global application-level signal arbiter.
pub fn app_events() -> &'static SignalArbiter {
  app_signals()
}

/// Error type for typed RPC calls.
///
/// This error type implements `std::error::Error` for integration with
/// error handling libraries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcError {
  /// No handler registered for the requested RPC method.
  NoHandler,
  /// The response type did not match the expected type.
  TypeMismatch,
}

impl std::fmt::Display for RpcError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::NoHandler => write!(f, "no handler registered for RPC method"),
      Self::TypeMismatch => write!(f, "RPC response type mismatch"),
    }
  }
}

impl std::error::Error for RpcError {}

/// Result type for RPC calls with explicit error reporting.
pub type RpcResult<T> = Result<T, RpcError>;

/// Error type for RPC calls with timeout support.
///
/// This error type implements `std::error::Error` for integration with
/// error handling libraries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RpcTimeoutError {
  /// The RPC call timed out before completing.
  Timeout,
  /// An RPC error occurred.
  Rpc(RpcError),
}

impl std::fmt::Display for RpcTimeoutError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Timeout => write!(f, "RPC call timed out"),
      Self::Rpc(err) => write!(f, "{err}"),
    }
  }
}

impl std::error::Error for RpcTimeoutError {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    match self {
      Self::Rpc(err) => Some(err),
      Self::Timeout => None,
    }
  }
}

impl From<RpcError> for RpcTimeoutError {
  #[inline]
  fn from(err: RpcError) -> Self {
    Self::Rpc(err)
  }
}

impl SignalArbiter {
  /// Creates a new, empty signal arbiter.
  pub fn new() -> Self {
    Self::default()
  }

  /// Sets the global broadcast capacity used for topic channels.
  ///
  /// This affects all newly created topics across all arbiters.
  pub fn set_global_broadcast_capacity(capacity: usize) {
    let cap = capacity.max(1);
    GLOBAL_BROADCAST_CAPACITY.store(cap, Ordering::SeqCst);
  }

  /// Returns the current global broadcast capacity.
  pub fn global_broadcast_capacity() -> usize {
    GLOBAL_BROADCAST_CAPACITY.load(Ordering::SeqCst)
  }

  /// Returns (and lazily initializes) the broadcast sender for a signal id.
  pub(crate) fn topic_sender(&self, id: &str) -> broadcast::Sender<Signal> {
    if let Some(existing) = self.inner.topics.get_sync(id) {
      existing.clone()
    } else {
      let cap = GLOBAL_BROADCAST_CAPACITY.load(Ordering::SeqCst);
      let (tx, _rx) = broadcast::channel(cap);
      let entry = self.inner.topics.entry_sync(id.to_string()).or_insert(tx);
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
      .entry_sync(id)
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

  /// Subscribes to all signals whose id starts with the given prefix.
  ///
  /// For example, `subscribe_prefix("request.")` will receive
  /// `request.started`, `request.completed`, etc.
  pub fn subscribe_prefix(&self, prefix: impl AsRef<str>) -> broadcast::Receiver<Signal> {
    let mut key = prefix.as_ref().to_string();
    if !key.ends_with('*') {
      key.push('*');
    }
    let sender = self.topic_sender(&key);
    sender.subscribe()
  }

  /// Subscribes to all signals regardless of their id.
  ///
  /// This is a special variant that receives every emitted signal.
  /// Internally uses a wildcard prefix matching (empty prefix = all signals).
  pub fn subscribe_all(&self) -> broadcast::Receiver<Signal> {
    self.subscribe_prefix("")
  }

  /// Broadcasts a signal to all subscribers without awaiting handler completion.
  pub(crate) fn broadcast(&self, signal: Signal) {
    // Exact id subscribers
    if let Some(sender) = self.inner.topics.get_sync(&signal.id) {
      let _ = sender.send(signal.clone());
    }

    // Prefix subscribers: keys ending with '*'
    self.inner.topics.iter_sync(|key, v| {
      if let Some(prefix) = key.strip_suffix('*') {
        if signal.id.starts_with(prefix) {
          let _ = v.send(signal.clone());
        }
      }

      true
    });
  }

  /// Subscribes using a filter function on top of an id-based subscription.
  ///
  /// This spawns a background task that forwards only matching signals into
  /// an unbounded channel, which is returned as a `SignalStream`.
  pub fn subscribe_filtered<F>(&self, id: impl AsRef<str>, filter: F) -> SignalStream
  where
    F: Fn(&Signal) -> bool + Send + Sync + 'static,
  {
    let mut rx = self.subscribe(id);
    let (tx, out_rx) = mpsc::unbounded_channel();
    let filter = Arc::new(filter);

    #[cfg(not(feature = "compio"))]
    tokio::spawn(async move {
      while let Ok(signal) = rx.recv().await {
        if filter(&signal) {
          if tx.send(signal).is_err() {
            break;
          }
        }
      }
    });

    #[cfg(feature = "compio")]
    compio::runtime::spawn(async move {
      while let Ok(signal) = rx.recv().await {
        if filter(&signal) {
          if tx.send(signal).is_err() {
            break;
          }
        }
      }
    })
    .detach();

    out_rx
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

    std::mem::drop(self.inner.rpc.insert_sync(id_str, handler));
  }

  /// Calls a typed RPC handler and returns a shared pointer to the response.
  pub async fn call_rpc_arc<Req, Res>(&self, id: impl AsRef<str>, req: Req) -> Option<Arc<Res>>
  where
    Req: Send + Sync + 'static,
    Res: Send + Sync + 'static,
  {
    let id_str = id.as_ref();
    let entry = self.inner.rpc.get_sync(id_str)?;
    let handler = entry.clone();
    drop(entry);

    let raw_req: Arc<dyn Any + Send + Sync> = Arc::new(req);
    let raw_res = handler(raw_req).await;

    match raw_res.downcast::<Res>() {
      Ok(res) => Some(res),
      Err(_) => None,
    }
  }

  /// Calls a typed RPC handler and returns an owned response with an error type.
  pub async fn call_rpc_result<Req, Res>(&self, id: impl AsRef<str>, req: Req) -> RpcResult<Res>
  where
    Req: Send + Sync + 'static,
    Res: Send + Sync + Clone + 'static,
  {
    let id_str = id.as_ref();
    let entry = self.inner.rpc.get_sync(id_str);
    let entry = match entry {
      Some(e) => e,
      None => return Err(RpcError::NoHandler),
    };
    let handler = entry.clone();
    drop(entry);

    let raw_req: Arc<dyn Any + Send + Sync> = Arc::new(req);
    let raw_res = handler(raw_req).await;

    match raw_res.downcast::<Res>() {
      Ok(res) => Ok((*res).clone()),
      Err(_) => Err(RpcError::TypeMismatch),
    }
  }

  /// Calls a typed RPC handler and returns an owned response.
  pub async fn call_rpc<Req, Res>(&self, id: impl AsRef<str>, req: Req) -> Option<Res>
  where
    Req: Send + Sync + 'static,
    Res: Send + Sync + Clone + 'static,
  {
    self.call_rpc_result::<Req, Res>(id, req).await.ok()
  }

  /// Calls a typed RPC handler with a timeout.
  #[cfg(not(feature = "compio"))]
  pub async fn call_rpc_timeout<Req, Res>(
    &self,
    id: impl AsRef<str>,
    req: Req,
    dur: Duration,
  ) -> Result<Res, RpcTimeoutError>
  where
    Req: Send + Sync + 'static,
    Res: Send + Sync + Clone + 'static,
  {
    match timeout(dur, self.call_rpc_result::<Req, Res>(id, req)).await {
      Ok(Ok(res)) => Ok(res),
      Ok(Err(e)) => Err(RpcTimeoutError::Rpc(e)),
      Err(_) => Err(RpcTimeoutError::Timeout),
    }
  }

  /// Emits a signal and awaits all registered handlers.
  ///
  /// Handlers run concurrently and this method resolves once all handlers have completed.
  pub async fn emit(&self, signal: Signal) {
    // First, broadcast to any subscribers.
    self.broadcast(signal.clone());

    // Call exporters (non-blocking from the perspective of handlers).
    self.inner.exporters.iter_sync(|_, v| {
      v(&signal);
      true
    });

    if let Some(entry) = self.inner.handlers.get_sync(&signal.id) {
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

  /// Registers a global exporter that is invoked for every emitted signal.
  ///
  /// Exporters are merged when routers are merged, similar to handlers.
  pub fn register_exporter<F>(&self, exporter: F)
  where
    F: Fn(&Signal) + Send + Sync + 'static,
  {
    // Use the pointer address as a simple, best-effort key.
    let key = Arc::into_raw(Arc::new(())) as u64;
    let exporter: SignalExporter = Arc::new(exporter);
    std::mem::drop(self.inner.exporters.insert_sync(key, exporter));
  }

  /// Merges all handlers from `other` into `self`.
  ///
  /// This is used by router merging so that signal handlers attached to
  /// a merged router continue to be active.
  pub(crate) fn merge_from(&self, other: &SignalArbiter) {
    other.inner.handlers.iter_sync(|k, v| {
      self
        .inner
        .handlers
        .entry_sync(k.clone())
        .or_insert_with(Vec::new)
        .extend(v.clone());

      true
    });

    other.inner.topics.iter_sync(|k, v| {
      self.inner.topics.entry_sync(k.clone()).or_insert(v.clone());
      true
    });

    other.inner.rpc.iter_sync(|k, v| {
      let _ = self.inner.rpc.insert_sync(k.clone(), v.clone());
      true
    });

    other.inner.exporters.iter_sync(|k, v| {
      let _ = self.inner.exporters.insert_sync(k.clone(), v.clone());
      true
    });
  }

  /// Returns a list of known signal ids (exact topics) currently registered.
  pub fn signal_ids(&self) -> Vec<String> {
    let mut ids = Vec::new();
    self.inner.topics.iter_sync(|k, _| {
      if !k.ends_with('*') {
        ids.push(k.clone());
      }
      true
    });
    ids
  }

  /// Returns a list of known signal prefixes (topics ending with '*').
  pub fn signal_prefixes(&self) -> Vec<String> {
    let mut prefixes = Vec::new();
    self.inner.topics.iter_sync(|k, _| {
      if k.ends_with('*') {
        prefixes.push(k.clone());
      }
      true
    });
    prefixes
  }

  /// Returns a list of registered RPC ids.
  pub fn rpc_ids(&self) -> Vec<String> {
    let mut ids = Vec::new();
    self.inner.rpc.iter_sync(|k, _| {
      ids.push(k.clone());
      true
    });
    ids
  }
}
