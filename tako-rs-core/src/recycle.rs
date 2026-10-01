//! Per-thread reuse of request header and extension maps.
//!
//! The framework hands a request's maps back once it no longer needs them and
//! builds later responses and requests on the same thread from them, so a
//! keep-alive connection serves requests without allocating either map.

use std::cell::Cell;
use std::cell::RefCell;
use std::net::SocketAddr;
use std::sync::Arc;

use http::Extensions;
use http::HeaderMap;
use http::request::Parts;

use crate::conn_info::ConnInfo;
use crate::extractors::body::BodyLimit;
use crate::extractors::params::PathParams;
use crate::router_state::MatchedPath;
use crate::router_state::RouterState;

/// Maps kept per thread; enough for the requests one worker has in flight.
const DEPTH: usize = 16;

thread_local! {
  static HEADERS: RefCell<Vec<HeaderMap>> = const { RefCell::new(Vec::new()) };
  static EXTENSIONS: RefCell<Vec<Extensions>> = const { RefCell::new(Vec::new()) };
  static ROUTER_ENTRIES: Cell<bool> = const { Cell::new(false) };
}

/// Returns a request's header and extension maps to this thread's pools.
pub fn recycle_parts(parts: Parts) {
  recycle_headers(parts.headers);
  recycle_extensions(parts.extensions);
}

/// Clears `headers` and keeps it for a later response on this thread.
pub fn recycle_headers(mut headers: HeaderMap) {
  if headers.capacity() == 0 {
    return;
  }
  headers.clear();
  HEADERS.with(|pool| {
    let mut pool = pool.borrow_mut();
    if pool.len() < DEPTH {
      pool.push(headers);
    }
  });
}

/// Takes a cleared header map from this thread's pool, or a new empty one.
pub fn take_headers() -> HeaderMap {
  HEADERS
    .with(|pool| pool.borrow_mut().pop())
    .unwrap_or_default()
}

/// Keeps `extensions` for a later request on this thread.
///
/// Entries that the HTTP/1 servers and the router overwrite on every request
/// stay, so the next request reuses their allocations. Router state is dropped
/// here, so a pooled map never keeps a stopped router's state alive, and any
/// other entry clears the map, so application values never reach another
/// request.
pub fn recycle_extensions(mut extensions: Extensions) {
  match framework_entries(&mut extensions) {
    Entries::Connection => {}
    Entries::Router => ROUTER_ENTRIES.with(|flag| flag.set(true)),
    Entries::Other => extensions.clear(),
  }
  EXTENSIONS.with(|pool| {
    let mut pool = pool.borrow_mut();
    if pool.len() < DEPTH {
      pool.push(extensions);
    }
  });
}

/// Takes an extension map from this thread's pool, or a new empty one.
///
/// The map may still hold the entries listed in [`recycle_extensions`]; the
/// caller must overwrite the connection entries before dispatch.
pub fn take_extensions() -> Extensions {
  EXTENSIONS
    .with(|pool| pool.borrow_mut().pop())
    .unwrap_or_default()
}

/// Whether a map recycled on this thread may hold a body limit, which the
/// router must then remove from requests that set none.
pub fn router_entries_pooled() -> bool {
  ROUTER_ENTRIES.with(Cell::get)
}

enum Entries {
  Connection,
  Router,
  Other,
}

fn framework_entries(extensions: &mut Extensions) -> Entries {
  let mut remaining = extensions.len();
  for has in [
    has::<ConnInfo>,
    has::<SocketAddr>,
    has::<MatchedPath>,
    has::<PathParams>,
  ] {
    if remaining == 0 {
      return Entries::Connection;
    }
    remaining -= usize::from(has(extensions));
  }
  if remaining > 0 && extensions.remove::<Arc<RouterState>>().is_some() {
    remaining -= 1;
  }
  if remaining == 0 {
    return Entries::Connection;
  }
  remaining -= usize::from(has::<BodyLimit>(extensions));
  if remaining == 0 {
    Entries::Router
  } else {
    Entries::Other
  }
}

fn has<T: Send + Sync + 'static>(extensions: &Extensions) -> bool {
  extensions.get::<T>().is_some()
}

#[cfg(test)]
mod tests {
  use std::net::SocketAddr;
  use std::sync::Arc;

  use http::Extensions;
  use http::HeaderMap;
  use http::HeaderValue;
  use http::header::HOST;

  use super::recycle_extensions;
  use super::recycle_headers;
  use super::recycle_parts;
  use super::router_entries_pooled;
  use super::take_extensions;
  use super::take_headers;
  use crate::conn_info::ConnInfo;
  use crate::extractors::body::BodyLimit;
  use crate::router_state::MatchedPath;
  use crate::router_state::RouterState;

  fn peer() -> SocketAddr {
    "127.0.0.1:4000".parse().unwrap()
  }

  #[test]
  fn recycled_header_map_comes_back_empty_with_its_capacity() {
    let mut headers = HeaderMap::with_capacity(16);
    headers.insert(HOST, HeaderValue::from_static("example.com"));
    let capacity = headers.capacity();
    recycle_headers(headers);
    let reused = take_headers();
    assert!(reused.is_empty());
    assert_eq!(reused.capacity(), capacity);
  }

  #[test]
  fn header_map_without_capacity_is_not_pooled() {
    recycle_headers(HeaderMap::new());
    recycle_headers(HeaderMap::with_capacity(4));
    assert!(take_headers().capacity() > 0);
    assert_eq!(take_headers().capacity(), 0);
  }

  #[test]
  fn extensions_with_only_framework_entries_keep_them_for_reuse() {
    let mut extensions = Extensions::new();
    extensions.insert(ConnInfo::tcp(peer()));
    extensions.insert(peer());
    extensions.insert(MatchedPath("/".into()));
    recycle_extensions(extensions);
    let reused = take_extensions();
    assert_eq!(reused.len(), 3);
    assert_eq!(reused.get::<MatchedPath>().unwrap().as_str(), "/");
  }

  #[test]
  fn body_limits_are_kept_and_flagged() {
    assert!(!router_entries_pooled());
    let mut extensions = Extensions::new();
    extensions.insert(ConnInfo::tcp(peer()));
    extensions.insert(BodyLimit(Some(64)));
    recycle_extensions(extensions);
    assert!(router_entries_pooled());
    assert_eq!(take_extensions().len(), 2);
  }

  #[test]
  fn recycling_releases_router_state() {
    let state = Arc::new(RouterState::new());
    let mut extensions = Extensions::new();
    extensions.insert(ConnInfo::tcp(peer()));
    extensions.insert(Arc::clone(&state));
    recycle_extensions(extensions);
    assert_eq!(Arc::strong_count(&state), 1);
    let reused = take_extensions();
    assert_eq!(reused.len(), 1);
    assert!(reused.get::<ConnInfo>().is_some());
  }

  #[test]
  fn extensions_with_application_entries_are_cleared() {
    let mut extensions = Extensions::new();
    extensions.insert(ConnInfo::tcp(peer()));
    extensions.insert(42_u32);
    recycle_extensions(extensions);
    assert!(take_extensions().is_empty());
  }

  #[test]
  fn pools_hold_a_bounded_number_of_maps() {
    for _ in 0..64 {
      recycle_headers(HeaderMap::with_capacity(4));
    }
    let pooled = std::iter::from_fn(|| Some(take_headers()))
      .take(64)
      .filter(|headers| headers.capacity() > 0)
      .count();
    assert!(pooled > 0 && pooled < 64);
  }

  #[test]
  fn recycling_request_parts_returns_both_maps() {
    let mut request = http::Request::new(());
    request
      .headers_mut()
      .insert(HOST, HeaderValue::from_static("example.com"));
    request.extensions_mut().insert(ConnInfo::tcp(peer()));
    recycle_parts(request.into_parts().0);
    assert!(take_headers().capacity() > 0);
    assert!(take_extensions().get::<ConnInfo>().is_some());
  }
}
