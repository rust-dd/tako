//! Read-only introspection over a [`Router`]'s registered routes.

use std::sync::Arc;
use std::sync::Weak;

use super::Router;
use crate::route::Route;

impl Router {
  /// Returns every route currently registered on this router.
  ///
  /// Routes come back grouped by HTTP method in the standard GET, POST, PUT,
  /// DELETE, PATCH, HEAD, OPTIONS, CONNECT, TRACE order, and within a method in
  /// registration order. Each path is the final registered path, so any prefix
  /// applied by [`Router::scope`] or [`Router::nest`] is already baked in — the
  /// returned set cannot drift from what the router actually dispatches, which
  /// is the whole point of asking the router instead of tracking routes
  /// separately.
  ///
  /// This is a cold-path accessor for startup-time inspection — route
  /// listings, reserved-namespace checks, generated documentation — not for the
  /// dispatch hot path.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::{router::Router, Method, types::Request};
  ///
  /// let mut router = Router::new();
  /// router.route(Method::GET, "/health", |_req: Request| async { "OK" });
  /// router.route(Method::POST, "/shorten", |_req: Request| async { "OK" });
  ///
  /// let paths: Vec<_> = router.routes().iter().map(|r| r.path.clone()).collect();
  /// assert_eq!(router.routes().len(), 2);
  /// assert!(paths.iter().any(|path| path.as_ref() == "/health"));
  /// ```
  #[must_use]
  pub fn routes(&self) -> Vec<Arc<Route>> {
    self
      .routes
      .iter()
      .flat_map(|(_, weak_vec)| weak_vec.iter())
      .filter_map(Weak::upgrade)
      .collect()
  }
}
