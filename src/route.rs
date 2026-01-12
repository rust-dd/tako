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

use std::collections::VecDeque;
use std::sync::Arc;
#[cfg(feature = "plugins")]
use std::sync::atomic::AtomicBool;
#[cfg(feature = "plugins")]
use std::sync::atomic::Ordering;

use http::Method;
use parking_lot::RwLock;

use crate::handler::BoxHandler;
use crate::middleware::Next;
#[cfg(any(feature = "utoipa", feature = "vespera"))]
use crate::openapi::RouteOpenApi;
#[cfg(feature = "plugins")]
use crate::plugins::TakoPlugin;
use crate::responder::Responder;
#[cfg(feature = "signals")]
use crate::signals::Signal;
#[cfg(feature = "signals")]
use crate::signals::SignalArbiter;
use crate::types::BoxMiddleware;
use crate::types::Request;

/// HTTP route with path pattern matching and middleware support.
#[doc(alias = "route")]
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
  /// Route-level signal arbiter.
  #[cfg(feature = "signals")]
  pub(crate) signals: SignalArbiter,
  /// OpenAPI metadata for this route.
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  pub(crate) openapi: RwLock<Option<RouteOpenApi>>,
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
      #[cfg(feature = "signals")]
      signals: SignalArbiter::new(),
      #[cfg(any(feature = "utoipa", feature = "vespera"))]
      openapi: RwLock::new(None),
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
  #[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
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
  #[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
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
  #[doc(alias = "tsr")]
  pub fn h2(&mut self) {
    self.http_protocol = Some(http::Version::HTTP_2);
  }

  /// Returns the configured protocol guard, if any.
  pub(crate) fn protocol_guard(&self) -> Option<http::Version> {
    self.http_protocol
  }

  #[cfg(feature = "signals")]
  /// Returns a reference to this route's signal arbiter.
  pub fn signals(&self) -> &SignalArbiter {
    &self.signals
  }

  #[cfg(feature = "signals")]
  /// Returns a clone of this route's signal arbiter for shared usage.
  pub fn signal_arbiter(&self) -> SignalArbiter {
    self.signals.clone()
  }

  #[cfg(feature = "signals")]
  /// Registers a handler for a named signal on this route's arbiter.
  pub fn on_signal<F, Fut>(&self, id: impl Into<String>, handler: F)
  where
    F: Fn(Signal) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
  {
    self.signals.on(id, handler);
  }

  #[cfg(feature = "signals")]
  /// Emits a signal through this route's arbiter.
  pub async fn emit_signal(&self, signal: Signal) {
    self.signals.emit(signal).await;
  }

  // ─────────────────────────────────────────────────────────────────────────────
  // OpenAPI metadata methods
  // ─────────────────────────────────────────────────────────────────────────────

  /// Sets a unique operation ID for this route in OpenAPI documentation.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// router.route(Method::GET, "/users", list_users)
  ///     .operation_id("listUsers");
  /// ```
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn operation_id(&self, id: impl Into<String>) -> &Self {
    let mut guard = self.openapi.write();
    let openapi = guard.get_or_insert_with(RouteOpenApi::default);
    openapi.operation_id = Some(id.into());
    self
  }

  /// Sets a short summary for this route in OpenAPI documentation.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// router.route(Method::GET, "/users/{id}", get_user)
  ///     .summary("Get user by ID");
  /// ```
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn summary(&self, summary: impl Into<String>) -> &Self {
    let mut guard = self.openapi.write();
    let openapi = guard.get_or_insert_with(RouteOpenApi::default);
    openapi.summary = Some(summary.into());
    self
  }

  /// Sets a detailed description for this route in OpenAPI documentation.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// router.route(Method::GET, "/users/{id}", get_user)
  ///     .description("Retrieves a user by their unique identifier");
  /// ```
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn description(&self, description: impl Into<String>) -> &Self {
    let mut guard = self.openapi.write();
    let openapi = guard.get_or_insert_with(RouteOpenApi::default);
    openapi.description = Some(description.into());
    self
  }

  /// Adds a tag to group this route in OpenAPI documentation.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// router.route(Method::GET, "/users", list_users)
  ///     .tag("users")
  ///     .tag("public");
  /// ```
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn tag(&self, tag: impl Into<String>) -> &Self {
    let mut guard = self.openapi.write();
    let openapi = guard.get_or_insert_with(RouteOpenApi::default);
    openapi.tags.push(tag.into());
    self
  }

  /// Marks this route as deprecated in OpenAPI documentation.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// router.route(Method::GET, "/v1/users", list_users_v1)
  ///     .deprecated();
  /// ```
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn deprecated(&self) -> &Self {
    let mut guard = self.openapi.write();
    let openapi = guard.get_or_insert_with(RouteOpenApi::default);
    openapi.deprecated = true;
    self
  }

  /// Adds a response description for a status code in OpenAPI documentation.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// router.route(Method::GET, "/users/{id}", get_user)
  ///     .response(200, "Successful response with user data")
  ///     .response(404, "User not found");
  /// ```
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn response(&self, status: u16, description: impl Into<String>) -> &Self {
    let mut guard = self.openapi.write();
    let openapi = guard.get_or_insert_with(RouteOpenApi::default);
    openapi.responses.insert(status, description.into());
    self
  }

  /// Adds a parameter definition for this route in OpenAPI documentation.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// use tako::openapi::{OpenApiParameter, ParameterLocation};
  ///
  /// router.route(Method::GET, "/users", list_users)
  ///     .parameter(OpenApiParameter {
  ///         name: "limit".to_string(),
  ///         location: ParameterLocation::Query,
  ///         description: Some("Maximum number of results".to_string()),
  ///         required: false,
  ///     });
  /// ```
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn parameter(&self, param: crate::openapi::OpenApiParameter) -> &Self {
    let mut guard = self.openapi.write();
    let openapi = guard.get_or_insert_with(RouteOpenApi::default);
    openapi.parameters.push(param);
    self
  }

  /// Sets the request body description for this route in OpenAPI documentation.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// use tako::openapi::OpenApiRequestBody;
  ///
  /// router.route(Method::POST, "/users", create_user)
  ///     .request_body(OpenApiRequestBody {
  ///         description: Some("User data to create".to_string()),
  ///         required: true,
  ///         content_type: "application/json".to_string(),
  ///     });
  /// ```
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn request_body(&self, body: crate::openapi::OpenApiRequestBody) -> &Self {
    let mut guard = self.openapi.write();
    let openapi = guard.get_or_insert_with(RouteOpenApi::default);
    openapi.request_body = Some(body);
    self
  }

  /// Adds a security requirement for this route in OpenAPI documentation.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// router.route(Method::DELETE, "/users/{id}", delete_user)
  ///     .security("bearerAuth");
  /// ```
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn security(&self, requirement: impl Into<String>) -> &Self {
    let mut guard = self.openapi.write();
    let openapi = guard.get_or_insert_with(RouteOpenApi::default);
    openapi.security.push(requirement.into());
    self
  }

  /// Returns a clone of the OpenAPI metadata for this route, if any.
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn openapi_metadata(&self) -> Option<RouteOpenApi> {
    self.openapi.read().clone()
  }
}
