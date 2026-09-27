//! Signal event type, typed-payload trait, and well-known identifiers.

use std::any::Any;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use tokio::sync::mpsc;

use crate::types::BuildHasher;

/// Well-known signal identifiers for common lifecycle and request events.
///
/// Request and route events are delivered to the owning router and the app arbiter.
/// Route events additionally reach the matching route's arbiter. Router-local
/// listeners are suitable for per-service metrics; app listeners observe all routers.
/// Connection and server lifecycle events are emitted on the app arbiter.
pub mod ids {
  pub const SERVER_STARTED: &str = "server.started";
  pub const SERVER_STOPPED: &str = "server.stopped";
  pub const CONNECTION_OPENED: &str = "connection.opened";
  pub const CONNECTION_CLOSED: &str = "connection.closed";
  pub const REQUEST_STARTED: &str = "request.started";
  pub const REQUEST_COMPLETED: &str = "request.completed";
  /// Reserved for explicit application emission; router changes do not emit this event.
  pub const ROUTER_HOT_RELOAD: &str = "router.hot_reload";
  pub const RPC_ERROR: &str = "rpc.error";
  pub const ROUTE_REQUEST_STARTED: &str = "route.request.started";
  pub const ROUTE_REQUEST_COMPLETED: &str = "route.request.completed";
}

/// Experimental bridge contract; not connected to router or app arbiters.
#[doc(hidden)]
pub mod bus {
  use async_trait::async_trait;

  use super::Signal;

  /// Inbound + outbound bridge between an in-process arbiter and a remote
  /// pub/sub topic. Implementations should be cheap to clone (`Arc`-based).
  #[async_trait]
  pub trait SignalBus: Send + Sync + 'static {
    /// Publish a signal to the remote topic.
    async fn publish(&self, signal: &Signal);
  }

  /// No-op bus — every published signal is dropped. Useful as a default and
  /// in tests.
  #[derive(Clone, Default)]
  pub struct LocalBus;

  #[async_trait]
  impl SignalBus for LocalBus {
    async fn publish(&self, _signal: &Signal) {}
  }
}

/// A signal emitted through the arbiter.
///
/// Signals are identified by an arbitrary string and can carry a map of
/// metadata. Callers are free to define their own conventions for ids and
/// fields.
#[derive(Clone, Debug, Default)]
pub struct Signal {
  /// Identifier of the signal, for example "request.started" or "metrics.tick".
  pub id: Cow<'static, str>,
  /// Optional metadata payload carried with the signal.
  pub metadata: HashMap<Cow<'static, str>, String, BuildHasher>,
}

impl Signal {
  /// Creates a new signal with the given id and empty metadata.
  #[inline]
  #[must_use]
  pub fn new(id: impl Into<Cow<'static, str>>) -> Self {
    Self {
      id: id.into(),
      metadata: HashMap::with_hasher(BuildHasher::default()),
    }
  }

  /// Creates a signal with pre-allocated capacity for the given number of metadata entries.
  #[inline]
  #[must_use]
  pub fn with_capacity(id: impl Into<Cow<'static, str>>, capacity: usize) -> Self {
    Self {
      id: id.into(),
      metadata: HashMap::with_capacity_and_hasher(capacity, BuildHasher::default()),
    }
  }

  /// Adds a metadata entry, returning self for chaining.
  #[inline]
  pub fn meta(mut self, key: impl Into<Cow<'static, str>>, value: impl Into<String>) -> Self {
    self.metadata.insert(key.into(), value.into());
    self
  }

  /// Creates a new signal with initial metadata.
  #[inline]
  #[must_use]
  pub fn with_metadata(
    id: impl Into<Cow<'static, str>>,
    metadata: HashMap<String, String, BuildHasher>,
  ) -> Self {
    Self {
      id: id.into(),
      metadata: metadata
        .into_iter()
        .map(|(key, value)| (Cow::Owned(key), value))
        .collect(),
    }
  }

  /// Creates a signal from a typed payload implementing `SignalPayload`.
  #[inline]
  #[must_use]
  pub fn from_payload<P: SignalPayload>(payload: &P) -> Self {
    Self {
      id: Cow::Borrowed(payload.id()),
      metadata: payload
        .to_metadata()
        .into_iter()
        .map(|(key, value)| (Cow::Owned(key), value))
        .collect(),
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
///
/// Bounded with [`FILTERED_SUBSCRIPTION_BUFFER`] capacity: a slow consumer
/// no longer accumulates an unbounded backlog (OOM risk). When the queue is
/// full, the producer-side forwarder drops the signal rather than blocking.
pub type SignalStream = mpsc::Receiver<Signal>;

/// Bounded buffer size for [`SignalArbiter::subscribe_filtered`](super::SignalArbiter::subscribe_filtered). Picked to
/// absorb short bursts while keeping per-subscriber memory bounded (~1024
/// `Signal`s ≈ 64 KiB plus per-signal metadata). Slow consumers experience
/// overflow as silent drops, not OOM.
pub const FILTERED_SUBSCRIPTION_BUFFER: usize = 1024;

/// Upper bound for [`SignalArbiter::set_global_broadcast_capacity`](super::SignalArbiter::set_global_broadcast_capacity). Caps each
/// new topic channel at 1 Mi elements so a misconfigured caller cannot ask
/// for a `usize::MAX`-element channel that would OOM the process on the next
/// topic creation.
pub const MAX_BROADCAST_CAPACITY: usize = 1 << 20;
