use std::any::Any;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use tokio::sync::mpsc;

use crate::types::BuildHasher;

/// Built-in signal ids. Request and route events reach the owning router and the app arbiter
/// (route events also the route's); server and connection events only the app arbiter.
pub mod ids {
  pub const SERVER_STARTED: &str = "server.started";
  pub const SERVER_STOPPED: &str = "server.stopped";
  pub const CONNECTION_OPENED: &str = "connection.opened";
  pub const CONNECTION_CLOSED: &str = "connection.closed";
  pub const REQUEST_STARTED: &str = "request.started";
  pub const REQUEST_COMPLETED: &str = "request.completed";
  /// Reserved for explicit application emission; router changes do not emit this event.
  pub const ROUTER_HOT_RELOAD: &str = "router.hot_reload";
  pub const ROUTE_REQUEST_STARTED: &str = "route.request.started";
  pub const ROUTE_REQUEST_COMPLETED: &str = "route.request.completed";
}

/// A named event with string metadata, emitted through a
/// [`SignalArbiter`](super::SignalArbiter).
#[derive(Clone, Debug, Default)]
pub struct Signal {
  /// For example `request.started` or `metrics.tick`.
  pub id: Cow<'static, str>,
  pub metadata: HashMap<Cow<'static, str>, String, BuildHasher>,
}

impl Signal {
  #[inline]
  #[must_use]
  pub fn new(id: impl Into<Cow<'static, str>>) -> Self {
    Self {
      id: id.into(),
      metadata: HashMap::with_hasher(BuildHasher::default()),
    }
  }

  #[inline]
  #[must_use]
  pub fn with_capacity(id: impl Into<Cow<'static, str>>, capacity: usize) -> Self {
    Self {
      id: id.into(),
      metadata: HashMap::with_capacity_and_hasher(capacity, BuildHasher::default()),
    }
  }

  #[inline]
  pub fn meta(mut self, key: impl Into<Cow<'static, str>>, value: impl Into<String>) -> Self {
    self.metadata.insert(key.into(), value.into());
    self
  }

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

/// A typed event, turned into a [`Signal`] by [`Signal::from_payload`].
pub trait SignalPayload {
  fn id(&self) -> &'static str;

  fn to_metadata(&self) -> HashMap<String, String, BuildHasher>;
}

pub type SignalHandler = Arc<dyn Fn(Signal) -> BoxFuture<'static, ()> + Send + Sync>;

/// Type-erased RPC handler; request and response are downcast at call time.
pub type RpcHandler = Arc<
  dyn Fn(Arc<dyn Any + Send + Sync>) -> BoxFuture<'static, Arc<dyn Any + Send + Sync>>
    + Send
    + Sync,
>;

/// Callback run for every emitted signal.
pub type SignalExporter = Arc<dyn Fn(&Signal) + Send + Sync>;

/// Filtered subscription holding up to [`FILTERED_SUBSCRIPTION_BUFFER`] signals; new ones
/// are dropped while it is full.
pub type SignalStream = mpsc::Receiver<Signal>;

/// Capacity of a [`SignalStream`], so a slow consumer loses signals instead of memory.
pub const FILTERED_SUBSCRIPTION_BUFFER: usize = 1024;

/// Ceiling for [`set_global_broadcast_capacity`].
///
/// [`set_global_broadcast_capacity`]: super::SignalArbiter::set_global_broadcast_capacity
pub const MAX_BROADCAST_CAPACITY: usize = 1 << 20;
