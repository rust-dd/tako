//! Per-router typed state; `State<T>` reads it before falling back to the process-global
//! [`get_state`](crate::state::get_state).

use std::any::Any;
use std::any::TypeId;
use std::sync::Arc;

use scc::HashMap as SccHashMap;

use crate::extractors::Entries;

/// The URI at the first router entry, preserved when middleware or a nested dispatch rewrites it.
#[derive(Debug, Clone)]
pub struct OriginalUriMarker(pub http::Uri);

/// Values keyed by type; a nested route's state also searches the states it was layered on.
#[derive(Default)]
pub struct RouterState {
  inner: SccHashMap<TypeId, Arc<dyn Any + Send + Sync>>,
  parents: Vec<Arc<RouterState>>,
}

impl std::fmt::Debug for RouterState {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("RouterState").finish_non_exhaustive()
  }
}

impl RouterState {
  #[must_use]
  pub fn new() -> Self {
    Self::default()
  }

  pub(crate) fn layered(child: Arc<Self>, parent: Arc<Self>) -> Self {
    Self {
      inner: SccHashMap::new(),
      parents: vec![child, parent],
    }
  }

  /// Insert (or replace) the value associated with `T`.
  pub fn insert<T: Send + Sync + 'static>(&self, value: T) {
    self.inner.upsert_sync(TypeId::of::<T>(), Arc::new(value));
  }

  /// Returns the `T`, looking through the layered states when this one has none.
  pub fn get<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
    self
      .inner
      .read_sync(&TypeId::of::<T>(), |_, v| Arc::clone(v))
      .and_then(|v| v.downcast::<T>().ok())
      .or_else(|| self.parents.iter().find_map(|parent| parent.get::<T>()))
  }

  pub fn is_empty(&self) -> bool {
    self.inner.is_empty() && self.parents.iter().all(|parent| parent.is_empty())
  }

  /// Number of distinct types currently stored.
  pub fn len(&self) -> usize {
    let mut types = std::collections::HashSet::new();
    self.collect_types(&mut types);
    types.len()
  }
  fn collect_types(&self, types: &mut std::collections::HashSet<TypeId>) {
    self.inner.iter_sync(|key, _| {
      types.insert(*key);
      true
    });
    for parent in &self.parents {
      parent.collect_types(types);
    }
  }
}

/// The matched route template, such as `/users/{id}`, as a request extension and extractor.
#[derive(Debug, Clone)]
pub struct MatchedPath(pub Arc<str>);

impl MatchedPath {
  #[inline]
  pub fn as_str(&self) -> &str {
    &self.0
  }
}

impl<'a> crate::extractors::FromRequest<'a> for MatchedPath {
  type Error = MatchedPathMissing;
  const ENTRIES: Entries = Entries::MATCHED_PATH;

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
  const ENTRIES: Entries = Entries::MATCHED_PATH;

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
