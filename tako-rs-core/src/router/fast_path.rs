//! Synchronous route matching that starts plain handlers without the full
//! dispatch pipeline.
//!
//! The entry helpers also rewrite the per-request entries in place, so a request
//! built on a recycled extension map reuses its allocations and never sees the
//! entries of the request that used the map before.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use http::Extensions;
use http::Method;

use super::Router;
use crate::conn_info::ConnInfo;
use crate::extractors::Entries;
use crate::extractors::body::BodyLimit;
use crate::extractors::params::PathParams;
use crate::handler::HandlerFuture;
use crate::recycle;
use crate::route::Route;
use crate::router_state::MatchedPath;
use crate::router_state::RouterState;
use crate::types::Request;

/// How [`Router::begin`] handled a request.
// Built and matched on the stack right away; boxing the request would cost an
// allocation on every full-pipeline request.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Dispatch {
  /// The matched route's handler is running; the future yields its response.
  Handler(HandlerFuture),
  /// The request needs middleware, a timeout, a response hook, or a fallback;
  /// finish it with [`Router::dispatch_full`].
  Full(Request),
}

impl Router {
  /// Matches `req` and starts the handler right away when neither the router
  /// nor the route adds middleware, timeouts, guards, signals, or error hooks.
  ///
  /// `peer` is set by HTTP/1 TCP servers, which leave the connection entries
  /// to the router; a plain route then gets only the entries its handler's
  /// extractors list.
  pub(crate) fn begin(&self, req: Request, peer: Option<SocketAddr>) -> Dispatch {
    if req.method() == Method::HEAD || !self.serves_plain_routes() {
      return Dispatch::Full(with_conn(req, peer));
    }
    let (mut parts, body) = req.into_parts();
    let Some(matched) = self
      .inner
      .get(&parts.method)
      .and_then(|routes| routes.at(parts.uri.path()).ok())
    else {
      return Dispatch::Full(with_conn(Request::from_parts(parts, body), peer));
    };
    let route: &Route = matched.value;
    if !self.is_plain(route) {
      return Dispatch::Full(with_conn(Request::from_parts(parts, body), peer));
    }
    let needs = route.handler.entries();
    if needs == Entries::ALL {
      if let Some(peer) = peer {
        attach_conn(&mut parts.extensions, peer);
      }
      set_params(&mut parts.extensions, route, &matched.params);
      set_matched_path(&mut parts.extensions, route);
      self.set_route_entries(&mut parts.extensions, Some(route));
    } else if needs != Entries::NONE {
      self.attach_listed(&mut parts.extensions, route, &matched.params, peer, needs);
    }
    Dispatch::Handler(route.handler.call(Request::from_parts(parts, body)))
  }

  /// Attaches the listed entries; on a recycled map it also drops the
  /// framework entries of an earlier request that `needs` leaves out.
  fn attach_listed(
    &self,
    extensions: &mut Extensions,
    route: &Route,
    params: &matchit::Params<'_, '_>,
    peer: Option<SocketAddr>,
    needs: Entries,
  ) {
    let recycled = peer.is_some() && extensions.is_empty();
    if recycled {
      *extensions = recycle::take_extensions();
    }
    let mut set = Entries::NONE;
    let mut count = 0;
    if let Some(peer) = peer.filter(|_| needs.contains(Entries::CONN)) {
      set_conn(extensions, peer);
      set = set.union(Entries::CONN);
      count += 2;
    }
    if needs.contains(Entries::PARAMS) && !params.is_empty() {
      set_params(extensions, route, params);
      set = set.union(Entries::PARAMS);
      count += 1;
    }
    if needs.contains(Entries::MATCHED_PATH) {
      set_matched_path(extensions, route);
      set = set.union(Entries::MATCHED_PATH);
      count += 1;
    }
    if needs.contains(Entries::STATE)
      && let Some(state) = self.state_for(Some(route))
    {
      set_state(extensions, state);
      count += 1;
    }
    if needs.contains(Entries::BODY_LIMIT)
      && let Some(limit) = route.body_limit.or(self.body_limit)
    {
      *extensions.get_or_insert(limit) = limit;
      set = set.union(Entries::BODY_LIMIT);
      count += 1;
    }
    if needs.contains(Entries::SIMD_JSON)
      && let Some(mode) = route.get_simd_json_mode()
    {
      extensions.insert(mode);
      count += 1;
    }
    if recycled && extensions.len() > count {
      drop_unset(extensions, set);
    }
  }

  fn serves_plain_routes(&self) -> bool {
    #[cfg(feature = "plugins")]
    if self.setup_plugins_once().is_err() {
      return false;
    }
    !self.has_global_middleware.load(Ordering::Acquire)
      && self.error_handler.is_none()
      && self.client_error_handler.is_none()
      && self.error_handler_with_parts.is_none()
  }

  fn is_plain(&self, route: &Route) -> bool {
    #[cfg(feature = "plugins")]
    if route.setup_plugins_once().is_err() {
      return false;
    }
    #[cfg(feature = "signals")]
    if super::request_signals::listening(&self.signals, Some(route)) {
      return false;
    }
    let timeout_router = route.scoped_timeout.as_deref().unwrap_or(self);
    !route.has_middleware.load(Ordering::Acquire)
      && route.protocol_guard().is_none()
      && route
        .get_timeout()
        .or(timeout_router.timeout)
        .or(self.timeout)
        .is_none()
  }

  /// Sets the state and body-limit entries for `route`, or for an unmatched
  /// request when `route` is `None`. Router state is never left over from a
  /// recycled map, since recycling drops it.
  pub(super) fn set_route_entries(&self, extensions: &mut Extensions, route: Option<&Route>) {
    if let Some(state) = self.state_for(route) {
      set_state(extensions, state);
    }
    match route.and_then(|route| route.body_limit).or(self.body_limit) {
      Some(limit) => *extensions.get_or_insert(limit) = limit,
      None if crate::recycle::router_entries_pooled() => {
        extensions.remove::<BodyLimit>();
      }
      None => {}
    }
    if let Some(mode) = route.and_then(Route::get_simd_json_mode) {
      extensions.insert(mode);
    }
  }
}

impl Router {
  fn state_for<'r>(&'r self, route: Option<&'r Route>) -> Option<&'r Arc<RouterState>> {
    route
      .and_then(|route| route.scoped_state.as_ref())
      .or_else(|| {
        self
          .has_router_state
          .load(Ordering::Acquire)
          .then_some(&self.router_state)
      })
  }
}

/// Adds the connection entries for a full-pipeline request.
fn with_conn(mut req: Request, peer: Option<SocketAddr>) -> Request {
  if let Some(peer) = peer {
    attach_conn(req.extensions_mut(), peer);
  }
  req
}

/// Adds `ConnInfo` and the peer address, building on a recycled map unless the
/// transport already attached entries, such as hyper's upgrade handle.
pub(super) fn attach_conn(extensions: &mut Extensions, peer: SocketAddr) {
  if extensions.is_empty() {
    *extensions = recycle::take_extensions();
  }
  set_conn(extensions, peer);
}

fn set_conn(extensions: &mut Extensions, peer: SocketAddr) {
  match extensions.get_mut::<ConnInfo>() {
    Some(info) => *info = ConnInfo::tcp(peer),
    None => {
      extensions.insert(ConnInfo::tcp(peer));
    }
  }
  match extensions.get_mut::<SocketAddr>() {
    Some(addr) => *addr = peer,
    None => {
      extensions.insert(peer);
    }
  }
}

fn set_state(extensions: &mut Extensions, state: &Arc<RouterState>) {
  match extensions.get_mut::<Arc<RouterState>>() {
    Some(current) if Arc::ptr_eq(current, state) => {}
    Some(current) => *current = Arc::clone(state),
    None => {
      extensions.insert(Arc::clone(state));
    }
  }
}

/// Removes the recyclable entries that `set` does not include.
fn drop_unset(extensions: &mut Extensions, set: Entries) {
  if !set.contains(Entries::CONN) {
    extensions.remove::<ConnInfo>();
    extensions.remove::<SocketAddr>();
  }
  if !set.contains(Entries::PARAMS) {
    extensions.remove::<PathParams>();
  }
  if !set.contains(Entries::MATCHED_PATH) {
    extensions.remove::<MatchedPath>();
  }
  if !set.contains(Entries::BODY_LIMIT) {
    extensions.remove::<BodyLimit>();
  }
}

/// Stores the matched parameters, reusing a recycled entry and its strings.
pub(super) fn set_params(
  extensions: &mut Extensions,
  route: &Route,
  params: &matchit::Params<'_, '_>,
) {
  if params.is_empty() {
    extensions.remove::<PathParams>();
    return;
  }
  let stored = &mut extensions.get_or_insert_with(PathParams::default).0;
  stored.truncate(params.len());
  for (index, (key, value)) in params.iter().enumerate() {
    if let Some((stored_key, stored_value)) = stored.get_mut(index) {
      if **stored_key != *key {
        *stored_key = route_key(route, key);
      }
      stored_value.clear();
      stored_value.push_str(value);
    } else {
      stored.push((route_key(route, key), value.to_owned()));
    }
  }
}

/// Points the request's `MatchedPath` at `route`, reusing a recycled entry.
pub(super) fn set_matched_path(extensions: &mut Extensions, route: &Route) {
  match extensions.get_mut::<MatchedPath>() {
    Some(current) if *current.0 == *route.path => {}
    Some(current) => current.0 = route.matched_path(),
    None => {
      extensions.insert(MatchedPath(route.matched_path()));
    }
  }
}

/// Drops route entries a recycled map may carry into an unmatched request.
pub(super) fn clear_route_entries(extensions: &mut Extensions) {
  extensions.remove::<MatchedPath>();
  extensions.remove::<PathParams>();
}

fn route_key(route: &Route, key: &str) -> Arc<str> {
  route
    .parameter_keys
    .iter()
    .find(|candidate| candidate.as_ref() == key)
    .cloned()
    .unwrap_or_else(|| Arc::from(key))
}

#[cfg(test)]
mod tests;
