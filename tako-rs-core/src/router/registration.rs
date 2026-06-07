//! Route registration and HTTP-method builder shorthands.

use std::sync::Arc;

use http::Method;

use super::Router;
use crate::handler::BoxHandler;
use crate::handler::Handler;
use crate::route::Route;

impl Router {
  /// Registers a new route with the router.
  ///
  /// Associates an HTTP method and path pattern with a handler function. The path
  /// can contain dynamic segments using curly braces (e.g., `/users/{id}`), which
  /// are extracted as parameters during request processing.
  ///
  /// # Panics
  ///
  /// Panics if a route with the same method and path pattern is already registered.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::{router::Router, Method, responder::Responder, types::Request};
  ///
  /// async fn get_user(_req: Request) -> impl Responder {
  ///     "User details"
  /// }
  ///
  /// async fn create_user(_req: Request) -> impl Responder {
  ///     "User created"
  /// }
  ///
  /// let mut router = Router::new();
  /// router.route(Method::GET, "/users/{id}", get_user);
  /// router.route(Method::POST, "/users", create_user);
  /// router.route(Method::GET, "/health", |_req| async { "OK" });
  /// ```
  pub fn route<H, T>(&mut self, method: Method, path: &str, handler: H) -> Arc<Route>
  where
    H: Handler<T> + Clone + 'static,
  {
    let final_path = self.apply_pending_prefix(path);
    let route = Arc::new(Route::new(
      final_path.clone(),
      method.clone(),
      BoxHandler::new::<H, T>(handler),
      None,
    ));

    if let Err(err) = self
      .inner
      .get_or_default_mut(&method)
      .insert(final_path, route.clone())
    {
      panic!("Failed to register route: {err}");
    }

    self
      .routes
      .get_or_default_mut(&method)
      .push(Arc::downgrade(&route));

    route
  }

  /// Returns `path` with the active `pending_prefix` (if any) prepended.
  /// Cold path; only runs at registration time.
  pub(crate) fn apply_pending_prefix(&self, path: &str) -> String {
    match &self.pending_prefix {
      None => path.to_string(),
      Some(prefix) => {
        let prefix = prefix.trim_end_matches('/');
        if path.is_empty() || path == "/" {
          if prefix.is_empty() {
            "/".to_string()
          } else {
            prefix.to_string()
          }
        } else if path.starts_with('/') {
          let mut s = String::with_capacity(prefix.len() + path.len());
          s.push_str(prefix);
          s.push_str(path);
          s
        } else {
          let mut s = String::with_capacity(prefix.len() + 1 + path.len());
          s.push_str(prefix);
          s.push('/');
          s.push_str(path);
          s
        }
      }
    }
  }

  /// Registers a `GET` route. Shorthand for [`Router::route`] with [`Method::GET`].
  #[inline]
  pub fn get<H, T>(&mut self, path: &str, handler: H) -> Arc<Route>
  where
    H: Handler<T> + Clone + 'static,
  {
    self.route(Method::GET, path, handler)
  }

  /// Registers a `POST` route. Shorthand for [`Router::route`] with [`Method::POST`].
  #[inline]
  pub fn post<H, T>(&mut self, path: &str, handler: H) -> Arc<Route>
  where
    H: Handler<T> + Clone + 'static,
  {
    self.route(Method::POST, path, handler)
  }

  /// Registers a `PUT` route. Shorthand for [`Router::route`] with [`Method::PUT`].
  #[inline]
  pub fn put<H, T>(&mut self, path: &str, handler: H) -> Arc<Route>
  where
    H: Handler<T> + Clone + 'static,
  {
    self.route(Method::PUT, path, handler)
  }

  /// Registers a `DELETE` route. Shorthand for [`Router::route`] with [`Method::DELETE`].
  #[inline]
  pub fn delete<H, T>(&mut self, path: &str, handler: H) -> Arc<Route>
  where
    H: Handler<T> + Clone + 'static,
  {
    self.route(Method::DELETE, path, handler)
  }

  /// Registers a `PATCH` route. Shorthand for [`Router::route`] with [`Method::PATCH`].
  #[inline]
  pub fn patch<H, T>(&mut self, path: &str, handler: H) -> Arc<Route>
  where
    H: Handler<T> + Clone + 'static,
  {
    self.route(Method::PATCH, path, handler)
  }

  /// Registers a `HEAD` route. Shorthand for [`Router::route`] with [`Method::HEAD`].
  #[inline]
  pub fn head<H, T>(&mut self, path: &str, handler: H) -> Arc<Route>
  where
    H: Handler<T> + Clone + 'static,
  {
    self.route(Method::HEAD, path, handler)
  }

  /// Registers an `OPTIONS` route. Shorthand for [`Router::route`] with [`Method::OPTIONS`].
  #[inline]
  pub fn options<H, T>(&mut self, path: &str, handler: H) -> Arc<Route>
  where
    H: Handler<T> + Clone + 'static,
  {
    self.route(Method::OPTIONS, path, handler)
  }

  /// Registers a route with trailing slash redirection enabled.
  ///
  /// When TSR is enabled, requests to paths with or without trailing slashes
  /// are automatically redirected to the canonical version. This helps maintain
  /// consistent URLs and prevents duplicate content issues.
  ///
  /// # Panics
  ///
  /// - Panics if called with the root path (`"/"`) since TSR is not applicable.
  /// - Panics if a route with the same method and path pattern is already registered.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::{router::Router, Method, responder::Responder, types::Request};
  ///
  /// async fn api_handler(_req: Request) -> impl Responder {
  ///     "API endpoint"
  /// }
  ///
  /// let mut router = Router::new();
  /// // Both "/api" and "/api/" will redirect to the canonical form
  /// router.route_with_tsr(Method::GET, "/api", api_handler);
  /// ```
  pub fn route_with_tsr<H, T>(&mut self, method: Method, path: &str, handler: H) -> Arc<Route>
  where
    H: Handler<T> + Clone + 'static,
  {
    assert!(path != "/", "Cannot route with TSR for root path");

    let final_path = self.apply_pending_prefix(path);
    let route = Arc::new(Route::new(
      final_path.clone(),
      method.clone(),
      BoxHandler::new::<H, T>(handler),
      Some(true),
    ));

    if let Err(err) = self
      .inner
      .get_or_default_mut(&method)
      .insert(final_path, route.clone())
    {
      panic!("Failed to register route: {err}");
    }

    self
      .routes
      .get_or_default_mut(&method)
      .push(Arc::downgrade(&route));

    route
  }
}
