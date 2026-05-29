//! Per-router typed state container.
//!
//! Each [`crate::router::Router`] owns one [`RouterState`](crate::router_state::RouterState) (an `Arc<…>`
//! internally). Values inserted via [`crate::router::Router::with_state`] live
//! on the router instance — multiple `Router`s in the same process can hold
//! distinct state values for the same `T`, which the historical process-wide
//! [`crate::state::set_state`] cannot do.
//!
//! The `State` extractor (from `tako-extractors`) reads from the request-scoped
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
/// concrete URI (`/users/42`).
///
/// This is also the public extractor type — `tako-extractors` re-exports it
/// from this module so the extension key and the user-facing extractor share
/// one canonical type. Previously the extractor was a separate newtype with
/// the same name, which made it easy to insert the wrong type into request
/// extensions and have lookups silently miss.
#[derive(Debug, Clone)]
pub struct MatchedPath(pub String);

impl MatchedPath {
  /// Borrow the matched path template.
  #[inline]
  pub fn as_str(&self) -> &str {
    &self.0
  }
}

impl<'a> crate::extractors::FromRequest<'a> for MatchedPath {
  type Error = MatchedPathMissing;

  fn from_request(
    req: &'a mut crate::types::Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(
      req
        .extensions()
        .get::<MatchedPath>()
        .cloned()
        .ok_or(MatchedPathMissing),
    )
  }
}

impl<'a> crate::extractors::FromRequestParts<'a> for MatchedPath {
  type Error = MatchedPathMissing;

  fn from_request_parts(
    parts: &'a mut http::request::Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(
      parts
        .extensions
        .get::<MatchedPath>()
        .cloned()
        .ok_or(MatchedPathMissing),
    )
  }
}

/// Rejection when no [`MatchedPath`] extension is on the request.
#[derive(Debug)]
pub struct MatchedPathMissing;

impl crate::responder::Responder for MatchedPathMissing {
  fn into_response(self) -> crate::types::Response {
    let mut res = crate::types::Response::new(crate::body::TakoBody::from(
      "matched path is unavailable on this request",
    ));
    *res.status_mut() = http::StatusCode::INTERNAL_SERVER_ERROR;
    res
  }
}
