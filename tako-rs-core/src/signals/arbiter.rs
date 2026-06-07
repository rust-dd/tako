//! Shared signal arbiter: registry, subscription, dispatch, and RPC wiring.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use arc_swap::ArcSwap;
use futures_util::future::join_all;
use once_cell::sync::Lazy;
use scc::HashMap as SccHashMap;
use tokio::sync::broadcast;

use super::signal::RpcHandler;
use super::signal::Signal;
use super::signal::SignalExporter;
use super::signal::SignalHandler;

const DEFAULT_BROADCAST_CAPACITY: usize = 64;
static GLOBAL_BROADCAST_CAPACITY: AtomicUsize = AtomicUsize::new(DEFAULT_BROADCAST_CAPACITY);
static EXPORTER_KEY_COUNTER: AtomicU64 = AtomicU64::new(0);

type HandlerList = Arc<ArcSwap<Vec<SignalHandler>>>;

#[derive(Default)]
pub(crate) struct Inner {
  handlers: SccHashMap<String, HandlerList>,
  topics: SccHashMap<String, broadcast::Sender<Signal>>,
  pub(crate) rpc: SccHashMap<String, RpcHandler>,
  exporters: SccHashMap<u64, SignalExporter>,
}

fn new_handler_list() -> HandlerList {
  Arc::new(ArcSwap::new(Arc::new(Vec::new())))
}

/// Shared arbiter used to register and dispatch named signals.
#[derive(Clone, Default)]
pub struct SignalArbiter {
  pub(crate) inner: Arc<Inner>,
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

impl SignalArbiter {
  /// Creates a new, empty signal arbiter.
  pub fn new() -> Self {
    Self::default()
  }

  /// Sets the global broadcast capacity used for topic channels.
  ///
  /// This affects all newly created topics across all arbiters. The capacity
  /// is clamped to `[1, MAX_BROADCAST_CAPACITY]` (1 MiB-element ceiling) so a
  /// caller can never request a `usize::MAX`-element channel that would OOM
  /// on the next `topic_sender` allocation.
  pub fn set_global_broadcast_capacity(capacity: usize) {
    let cap = capacity.clamp(1, super::signal::MAX_BROADCAST_CAPACITY);
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
      Box::pin(fut)
    });

    let list = self.handler_list_for(id);
    list.rcu(|current| {
      let mut next = Vec::with_capacity(current.len() + 1);
      next.extend(current.iter().cloned());
      next.push(handler.clone());
      Arc::new(next)
    });
  }

  /// Returns (creating if necessary) the per-id `ArcSwap` holding the handler list.
  /// The SCC hashmap protects only the slot lookup; handler-list updates are
  /// wait-free via `ArcSwap` RCU, so concurrent `on()` / `emit()` calls cannot
  /// race on a `Vec::push` reallocation.
  fn handler_list_for(&self, id: String) -> HandlerList {
    let entry = self
      .inner
      .handlers
      .entry_sync(id)
      .or_insert_with(new_handler_list);
    entry.clone()
  }

  /// Subscribes to a broadcast channel for the given signal id.
  ///
  /// This is useful for long-lived listeners such as metrics collectors,
  /// background workers, plugins, or middleware driven tasks.
  ///
  /// ⚠️ **Topic retention:** the topic map is keyed by `id` and entries
  /// are created lazily on the first `subscribe`/`emit`. The `Sender` is
  /// retained for the lifetime of the arbiter — there is no TTL or LRU
  /// eviction. With **high-cardinality dynamic ids** (e.g. one signal id
  /// per session or per request) this is an unbounded slow-leak.
  ///
  /// Use a low-cardinality id set ("`request.started`", "`order.placed`")
  /// and put the per-request discriminator inside the [`Signal`] payload
  /// instead of the id string.
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

    // Prefix subscribers: keys ending with '*'.
    // Snapshot matching senders before sending so the SCC entry locks are
    // released by the time we deliver — even though `broadcast::Sender::send`
    // is non-blocking, this also bounds inconsistency to the moment of the
    // snapshot rather than spreading it across every per-entry send.
    let mut targets: Vec<broadcast::Sender<Signal>> = Vec::new();
    self.inner.topics.iter_sync(|key, v| {
      if let Some(prefix) = key.strip_suffix('*')
        && signal.id.starts_with(prefix)
      {
        targets.push(v.clone());
      }
      true
    });
    for sender in targets {
      let _ = sender.send(signal.clone());
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
        Err(broadcast::error::RecvError::Lagged(_)) => {}
        Err(_) => return None,
      }
    }
  }

  /// Emits a signal and awaits all registered handlers.
  ///
  /// Handlers run concurrently and this method resolves once all handlers have completed.
  pub async fn emit(&self, signal: Signal) {
    // First, broadcast to any subscribers.
    self.broadcast(signal.clone());

    // Call exporters asynchronously.
    self
      .inner
      .exporters
      .iter_async(|_, v| {
        v(&signal);
        true
      })
      .await;

    if let Some(entry) = self.inner.handlers.get_async(&signal.id).await {
      let list = entry.clone();
      drop(entry);
      let handlers = list.load_full();

      let futures = handlers.iter().map(|handler| {
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
    let key = EXPORTER_KEY_COUNTER.fetch_add(1, Ordering::Relaxed);
    let exporter: SignalExporter = Arc::new(exporter);
    // `upsert_sync` mirrors `register_rpc`: makes re-registration on the
    // same (very-rare) duplicate key a replace instead of a silent drop.
    self.inner.exporters.upsert_sync(key, exporter);
  }

  /// Merges all handlers from `other` into `self`.
  ///
  /// This is used by router merging so that signal handlers attached to
  /// a merged router continue to be active.
  pub(crate) fn merge_from(&self, other: &SignalArbiter) {
    other.inner.handlers.iter_sync(|k, other_list| {
      let other_handlers = other_list.load_full();
      if other_handlers.is_empty() {
        return true;
      }
      let target_list = self.handler_list_for(k.clone());
      target_list.rcu(|current| {
        let mut next = Vec::with_capacity(current.len() + other_handlers.len());
        next.extend(current.iter().cloned());
        next.extend(other_handlers.iter().cloned());
        Arc::new(next)
      });

      true
    });

    other.inner.topics.iter_sync(|k, v| {
      self.inner.topics.entry_sync(k.clone()).or_insert(v.clone());
      true
    });

    // On merge, the merged-in arbiter wins for conflicting ids/keys.
    // `insert_sync` would silently keep `self`'s entry, dropping the
    // intentional value the caller wanted to pull in.
    other.inner.rpc.iter_sync(|k, v| {
      self.inner.rpc.upsert_sync(k.clone(), v.clone());
      true
    });

    other.inner.exporters.iter_sync(|k, v| {
      self.inner.exporters.upsert_sync(*k, v.clone());
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
}
