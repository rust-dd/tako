//! Per-router typed state container.
//!
//! Each [`crate::router::Router`] owns one [`RouterState`] (an `Arc<…>`
//! internally). Values inserted via [`crate::router::Router::with_state`] live
//! on the router instance — multiple `Router`s in the same process can hold
//! distinct state values for the same `T`, which the historical process-wide
//! [`crate::state::set_state`] cannot do.
//!
//! [`crate::extractors::state::State`] reads from the request-scoped
//! `Arc<RouterState>` first (inserted by [`crate::router::Router::dispatch`])
//! and falls back to [`crate::state::get_state`] if the per-router slot is
//! empty. Existing code that uses the global store keeps working unchanged.

use std::any::Any;
use std::any::TypeId;
use std::sync::Arc;

use scc::HashMap as SccHashMap;

/// Type-keyed bag of values, lock-free for both reads and writes.
#[derive(Default)]
pub struct RouterState {
  inner: SccHashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl std::fmt::Debug for RouterState {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("RouterState").finish_non_exhaustive()
  }
}

impl RouterState {
  /// Construct an empty state container.
  #[must_use]
  pub fn new() -> Self {
    Self::default()
  }

  /// Insert (or replace) the value associated with `T`.
  pub fn insert<T: Send + Sync + 'static>(&self, value: T) {
    let _ = self.inner.insert_sync(TypeId::of::<T>(), Arc::new(value));
  }

  /// Retrieve the value associated with `T`, if any.
  pub fn get<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
    self
      .inner
      .get_sync(&TypeId::of::<T>())
      .map(|v| v.clone())
      .and_then(|v| v.downcast::<T>().ok())
  }

  /// `true` when at least one value has been inserted.
  pub fn is_empty(&self) -> bool {
    self.inner.is_empty()
  }

  /// Number of distinct types currently stored.
  pub fn len(&self) -> usize {
    self.inner.len()
  }
}

/// Routing-time path template attached to the request.
///
/// `Router::dispatch` inserts a `MatchedPath` into request extensions before
/// running middleware and the handler so that metrics, logs, and extractors
/// can label by the route template (e.g. `/users/{id}`) rather than the
/// concrete URI (`/users/42`). Use the `tako_extractors::matched_path::MatchedPath`
/// extractor in handlers — this struct is the underlying request extension.
#[derive(Debug, Clone)]
pub struct MatchedPath(pub String);

impl MatchedPath {
  /// Borrow the matched path template.
  #[inline]
  pub fn as_str(&self) -> &str {
    &self.0
  }
}
