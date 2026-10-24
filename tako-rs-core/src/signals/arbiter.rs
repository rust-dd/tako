use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use arc_swap::ArcSwap;
use futures_util::future::join_all;
use scc::HashMap as SccHashMap;
use tokio::sync::broadcast;

use super::signal::RpcHandler;
use super::signal::Signal;
use super::signal::SignalExporter;
use super::signal::SignalHandler;

#[cfg(test)]
mod lifetime_tests;
#[cfg(test)]
mod merge_tests;

const DEFAULT_BROADCAST_CAPACITY: usize = 64;
static GLOBAL_BROADCAST_CAPACITY: AtomicUsize = AtomicUsize::new(DEFAULT_BROADCAST_CAPACITY);
static EXPORTER_KEY_COUNTER: AtomicU64 = AtomicU64::new(0);

type HandlerList = Arc<ArcSwap<Vec<SignalHandler>>>;

#[derive(Default)]
pub(crate) struct Inner {
  handlers: SccHashMap<String, HandlerList>,
  has_listeners: AtomicBool,
  prefix_topics: ArcSwap<Vec<(String, broadcast::Sender<Signal>)>>,
  topics: SccHashMap<String, Vec<broadcast::Sender<Signal>>>,
  pub(crate) rpc: SccHashMap<String, RpcHandler>,
  exporters: SccHashMap<u64, SignalExporter>,
}

fn new_handler_list() -> HandlerList {
  Arc::new(ArcSwap::new(Arc::new(Vec::new())))
}

/// Dispatches signals to handlers, subscribers and exporters; clones share them.
#[derive(Clone, Default)]
pub struct SignalArbiter {
  pub(crate) inner: Arc<Inner>,
}

static APP_SIGNAL_ARBITER: LazyLock<SignalArbiter> = LazyLock::new(SignalArbiter::new);

/// The process-wide arbiter, which also receives the server, connection and request signals.
pub fn app_signals() -> &'static SignalArbiter {
  &APP_SIGNAL_ARBITER
}

impl SignalArbiter {
  pub fn new() -> Self {
    Self::default()
  }

  /// Whether anything ever listened here, so callers can skip building unused signals.
  pub fn has_listeners(&self) -> bool {
    self.inner.has_listeners.load(Ordering::Acquire)
  }

  /// Capacity of the topics created afterwards in every arbiter, clamped to
  /// `1..=`[`MAX_BROADCAST_CAPACITY`](super::MAX_BROADCAST_CAPACITY).
  pub fn set_global_broadcast_capacity(capacity: usize) {
    let cap = capacity.clamp(1, super::signal::MAX_BROADCAST_CAPACITY);
    GLOBAL_BROADCAST_CAPACITY.store(cap, Ordering::SeqCst);
  }

  pub fn global_broadcast_capacity() -> usize {
    GLOBAL_BROADCAST_CAPACITY.load(Ordering::SeqCst)
  }

  pub(crate) fn topic_sender(&self, id: &str) -> broadcast::Sender<Signal> {
    if let Some(existing) = self.inner.topics.get_sync(id) {
      existing[0].clone()
    } else {
      let cap = GLOBAL_BROADCAST_CAPACITY.load(Ordering::SeqCst);
      let (tx, _rx) = broadcast::channel(cap);
      let entry = self
        .inner
        .topics
        .entry_sync(id.to_string())
        .or_insert(vec![tx]);
      entry[0].clone()
    }
  }

  /// Registers a handler for `id`; [`emit`](Self::emit) runs an id's handlers concurrently.
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
    self.inner.has_listeners.store(true, Ordering::Release);
  }

  /// Handler lists are swapped by RCU, so concurrent `on` and `emit` never race on a push.
  fn handler_list_for(&self, id: String) -> HandlerList {
    let entry = self
      .inner
      .handlers
      .entry_sync(id)
      .or_insert_with(new_handler_list);
    entry.clone()
  }

  /// Subscribes to signals with exactly `id`. Each topic lives as long as the arbiter, so keep
  /// ids low-cardinality and put per-request data in the metadata.
  pub fn subscribe(&self, id: impl AsRef<str>) -> broadcast::Receiver<Signal> {
    let id_str = id.as_ref();
    let sender = self.topic_sender(id_str);
    let receiver = sender.subscribe();
    self.inner.has_listeners.store(true, Ordering::Release);
    receiver
  }

  /// Subscribes to every id starting with `prefix`; a trailing `*` is ignored.
  /// Dropped subscriptions are removed on subscription, emission, merge or introspection.
  pub fn subscribe_prefix(&self, prefix: impl AsRef<str>) -> broadcast::Receiver<Signal> {
    let key = prefix.as_ref().trim_end_matches('*').to_string();
    let (sender, receiver) = broadcast::channel(Self::global_broadcast_capacity());
    self.inner.prefix_topics.rcu(|current| {
      let mut next = current
        .iter()
        .filter(|(_, sender)| sender.receiver_count() != 0)
        .cloned()
        .collect::<Vec<_>>();
      next.push((key.clone(), sender.clone()));
      Arc::new(next)
    });
    self.inner.has_listeners.store(true, Ordering::Release);
    receiver
  }

  pub fn subscribe_all(&self) -> broadcast::Receiver<Signal> {
    self.subscribe_prefix("")
  }

  pub(crate) fn broadcast(&self, signal: Signal) {
    if let Some(senders) = self.inner.topics.get_sync(signal.id.as_ref()) {
      for sender in senders.iter() {
        let _ = sender.send(signal.clone());
      }
    }

    self.prune_prefix_topics();
    for (prefix, sender) in self.inner.prefix_topics.load().iter() {
      if signal.id.starts_with(prefix.as_str()) {
        let _ = sender.send(signal.clone());
      }
    }
  }

  fn prune_prefix_topics(&self) {
    if self
      .inner
      .prefix_topics
      .load()
      .iter()
      .all(|(_, sender)| sender.receiver_count() != 0)
    {
      return;
    }
    self.inner.prefix_topics.rcu(|current| {
      Arc::new(
        current
          .iter()
          .filter(|(_, sender)| sender.receiver_count() != 0)
          .cloned()
          .collect(),
      )
    });
  }

  /// Waits for the next signal with `id`; `None` once the topic closes.
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

  /// Sends `signal` to subscribers and exporters, then awaits its handlers, which run
  /// concurrently.
  pub async fn emit(&self, signal: Signal) {
    if !self.has_listeners() {
      return;
    }
    self.broadcast(signal.clone());

    self
      .inner
      .exporters
      .iter_async(|_, v| {
        v(&signal);
        true
      })
      .await;

    if let Some(entry) = self.inner.handlers.get_async(signal.id.as_ref()).await {
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

  /// Emits on [`app_signals`].
  pub async fn emit_app(signal: Signal) {
    app_signals().emit(signal).await;
  }

  /// Registers a callback run for every signal this arbiter emits; router merges keep it.
  pub fn register_exporter<F>(&self, exporter: F)
  where
    F: Fn(&Signal) + Send + Sync + 'static,
  {
    let key = EXPORTER_KEY_COUNTER.fetch_add(1, Ordering::Relaxed);
    let exporter: SignalExporter = Arc::new(exporter);
    self.inner.exporters.upsert_sync(key, exporter);
    self.inner.has_listeners.store(true, Ordering::Release);
  }

  /// Copies `other`'s handlers, subscriptions, RPCs and exporters into `self`; `other` wins
  /// when both register an RPC under one id. Existing subscribers on either arbiter keep
  /// receiving events; copying the same subscription or handler again never duplicates it.
  pub(crate) fn merge_from(&self, other: &SignalArbiter) {
    if Arc::ptr_eq(&self.inner, &other.inner) {
      return;
    }
    other.prune_prefix_topics();
    self.prune_prefix_topics();
    let prefixes = other.inner.prefix_topics.load_full();
    self.inner.prefix_topics.rcu(|current| {
      let mut next = (**current).clone();
      for (prefix, sender) in prefixes.iter() {
        if !next
          .iter()
          .any(|(_, existing)| existing.same_channel(sender))
        {
          next.push((prefix.clone(), sender.clone()));
        }
      }
      Arc::new(next)
    });
    other.inner.handlers.iter_sync(|k, other_list| {
      let other_handlers = other_list.load_full();
      if other_handlers.is_empty() {
        return true;
      }
      let target_list = self.handler_list_for(k.clone());
      target_list.rcu(|current| {
        let mut next = Vec::with_capacity(current.len() + other_handlers.len());
        next.extend(current.iter().cloned());
        for handler in other_handlers.iter() {
          if !next.iter().any(|existing| Arc::ptr_eq(existing, handler)) {
            next.push(handler.clone());
          }
        }
        Arc::new(next)
      });

      true
    });

    other.inner.topics.iter_sync(|k, v| {
      let mut entry = self
        .inner
        .topics
        .entry_sync(k.clone())
        .or_insert_with(Vec::new);
      for sender in v {
        if !entry.iter().any(|existing| existing.same_channel(sender)) {
          entry.get_mut().push(sender.clone());
        }
      }
      true
    });

    other.inner.rpc.iter_sync(|k, v| {
      self.inner.rpc.upsert_sync(k.clone(), v.clone());
      true
    });

    other.inner.exporters.iter_sync(|k, v| {
      self.inner.exporters.upsert_sync(*k, v.clone());
      true
    });
    if other.has_listeners() {
      self.inner.has_listeners.store(true, Ordering::Release);
    }
  }

  /// Ids of the exact-id topics created so far.
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

  /// Subscribed prefixes, each with a trailing `*`, sorted and deduplicated.
  pub fn signal_prefixes(&self) -> Vec<String> {
    self.prune_prefix_topics();
    let mut prefixes = self
      .inner
      .prefix_topics
      .load()
      .iter()
      .map(|(prefix, _)| format!("{prefix}*"))
      .collect::<Vec<_>>();
    prefixes.sort();
    prefixes.dedup();
    prefixes
  }
}
