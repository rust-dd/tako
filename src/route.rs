//! HTTP route definition and path matching functionality.
//!
//! This module provides the core `Route` struct for defining HTTP routes with path patterns,
//! parameter extraction, and middleware support. Routes can contain dynamic segments like
//! `{id}` that are captured as parameters, and support method-specific handlers with
//! optional trailing slash redirection and route-specific middleware chains.
//!
//! # Examples
//!
//! ```rust
//! use tako::route::Route;
//! use tako::handler::BoxHandler;
//! use tako::types::Request;
//! use http::Method;
//!
//! async fn handler(_req: Request) -> &'static str {
//!     "Hello, World!"
//! }
//!
//! let route = Route::new(
//!     "/users/{id}".to_string(),
//!     Method::GET,
//!     BoxHandler::new(handler),
//!     None
//! );
//!
//! let params = route.match_path("/users/123").unwrap();
//! assert_eq!(params.get("id"), Some(&"123".to_string()));
//! ```

use std::{collections::VecDeque, sync::Arc};

use http::Method;
use parking_lot::RwLock;

use crate::{
  handler::BoxHandler,
  middleware::Next,
  responder::Responder,
  types::{BoxMiddleware, Request},
};

#[cfg(feature = "plugins")]
use crate::plugins::TakoPlugin;

#[cfg(feature = "plugins")]
use std::sync::atomic::{AtomicBool, Ordering};

/// HTTP route with path pattern matching and middleware support.
pub struct Route {
  /// Original path string used to create this route.
  pub path: String,
  /// HTTP method this route responds to.
  pub method: Method,
  /// Handler function to execute when route is matched.
  pub handler: BoxHandler,
  /// Route-specific middleware chain.
  pub middlewares: RwLock<VecDeque<BoxMiddleware>>,
  /// Whether trailing slash redirection is enabled.
  pub tsr: bool,
  /// Route-specific plugins.
  #[cfg(feature = "plugins")]
  pub(crate) plugins: RwLock<Vec<Box<dyn TakoPlugin>>>,
  /// Flag to ensure route plugins are initialized only once.
  #[cfg(feature = "plugins")]
  plugins_initialized: AtomicBool,
  /// HTTP protocol version
  http_protocol: Option<http::Version>,
}

impl Route {
  /// Creates a new route with the specified path, method, and handler.
  pub fn new(path: String, method: Method, handler: BoxHandler, tsr: Option<bool>) -> Self {
    Self {
      path,
      method,
      handler,
      middlewares: RwLock::new(VecDeque::new()),
      tsr: tsr.unwrap_or(false),
      #[cfg(feature = "plugins")]
      plugins: RwLock::new(Vec::new()),
      #[cfg(feature = "plugins")]
      plugins_initialized: AtomicBool::new(false),
      http_protocol: None,
    }
  }

  /// Adds middleware to this route's execution chain.
  pub fn middleware<F, Fut, R>(&self, f: F) -> &Self
  where
    F: Fn(Request, Next) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = R> + Send + 'static,
    R: Responder + Send + 'static,
  {
    let mw: BoxMiddleware = Arc::new(move |req, next| {
      let fut = f(req, next); // Fut<'a>

      Box::pin(async move { fut.await.into_response() })
    });

    self.middlewares.write().push_back(mw);
    self
  }

  /// Adds a plugin to this route.
  ///
  /// Route-level plugins allow applying functionality like compression, CORS,
  /// or rate limiting to specific routes instead of globally. Plugins added
  /// to a route are initialized when the route is first accessed.
  ///
  /// # Examples
  ///
  /// ```rust
  /// # #[cfg(feature = "plugins")]
  /// use tako::{router::Router, Method, responder::Responder, types::Request};
  /// # #[cfg(feature = "plugins")]
  /// use tako::plugins::cors::CorsBuilder;
  ///
  /// # #[cfg(feature = "plugins")]
  /// # async fn handler(_req: Request) -> impl Responder {
  /// #     "Hello, World!"
  /// # }
  ///
  /// # #[cfg(feature = "plugins")]
  /// # fn example() {
  /// let mut router = Router::new();
  /// let route = router.route(Method::GET, "/api/data", handler);
  ///
  /// // Apply CORS only to this route
  /// let cors = CorsBuilder::new()
  ///     .allow_origin("https://example.com")
  ///     .build();
  /// route.plugin(cors);
  /// # }
  /// ```
  #[cfg(feature = "plugins")]
  pub fn plugin<P>(&self, plugin: P) -> &Self
  where
    P: TakoPlugin + Clone + Send + Sync + 'static,
  {
    self.plugins.write().push(Box::new(plugin));
    self
  }

  /// Initializes route-level plugins exactly once.
  ///
  /// This method sets up all plugins registered with this route by calling
  /// their setup method. It uses a mini-router to collect the middleware
  /// that plugins register, then adds that middleware to the route's
  /// middleware chain. This ensures plugins are only initialized once.
  #[cfg(feature = "plugins")]
  pub(crate) fn setup_plugins_once(&self) {
    if !self.plugins_initialized.swap(true, Ordering::SeqCst) {
      // Create a temporary mini-router to capture plugin middleware
      let mini_router = crate::router::Router::new();

      let plugins = self.plugins.read();
      for plugin in plugins.iter() {
        let _ = plugin.setup(&mini_router);
      }

      // Transfer middleware from mini-router to this route
      let plugin_middlewares = mini_router.middlewares.read();
      let mut route_middlewares = self.middlewares.write();

      // Prepend plugin middlewares to route middlewares
      for mw in plugin_middlewares.iter().rev() {
        route_middlewares.push_front(mw.clone());
      }
    }
  }

  /// HTTP/0.9 guard
  pub fn h09(&mut self) {
    self.http_protocol = Some(http::Version::HTTP_09);
  }

  /// HTTP/1.0 guard
  pub fn h10(&mut self) {
    self.http_protocol = Some(http::Version::HTTP_10);
  }

  /// HTTP/1.1 guard
  pub fn h11(&mut self) {
    self.http_protocol = Some(http::Version::HTTP_11);
  }

  /// HTTP/2 guard
  pub fn h2(&mut self) {
    self.http_protocol = Some(http::Version::HTTP_2);
  }

  /// Returns the configured protocol guard, if any.
  pub(crate) fn protocol_guard(&self) -> Option<http::Version> {
    self.http_protocol
  }
}
