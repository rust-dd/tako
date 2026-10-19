//! Read-only introspection over a [`Router`]'s registered routes.

use std::sync::Arc;
use std::sync::Weak;

use super::Router;
use crate::route::Route;

impl Router {
  /// Every registered route with its final, prefixed path: grouped by method (GET, POST, …
  /// first), then in registration order. Allocates, so keep it off the hot path.
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
