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

use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use arc_swap::ArcSwap;
use http::Method;
#[cfg(any(feature = "plugins", feature = "utoipa", feature = "vespera"))]
use parking_lot::RwLock;

use crate::extractors::json::SimdJsonMode;
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
  pub middlewares: ArcSwap<Vec<BoxMiddleware>>,
  /// Fast check: true when route middleware is registered (avoids ArcSwap load on hot path).
  pub(crate) has_middleware: AtomicBool,
  /// Whether trailing slash redirection is enabled.
  pub tsr: bool,
  /// Route-specific plugins.
  #[cfg(feature = "plugins")]
  pub(crate) plugins: RwLock<Vec<Box<dyn TakoPlugin>>>,
  /// Flag to ensure route plugins are initialized only once.
  #[cfg(feature = "plugins")]
  plugins_initialized: AtomicBool,
  /// HTTP protocol version guard (set once via [`Route::version`] / `h09`/`h10`/`h11`/`h2`).
  http_protocol: OnceLock<http::Version>,
  /// Route-level signal arbiter.
  #[cfg(feature = "signals")]
  pub(crate) signals: SignalArbiter,
  /// OpenAPI metadata for this route.
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  pub(crate) openapi: RwLock<Option<RouteOpenApi>>,
  /// Route-specific timeout override (set once at registration, lock-free reads).
  pub(crate) timeout: OnceLock<Duration>,
  /// Route-level SIMD JSON dispatch mode (set once at registration, lock-free reads).
  pub(crate) simd_json_mode: OnceLock<SimdJsonMode>,
}

impl Route {
  /// Creates a new route with the specified path, method, and handler.
  pub fn new(path: String, method: Method, handler: BoxHandler, tsr: Option<bool>) -> Self {
    Self {
      path,
      method,
      handler,
      middlewares: ArcSwap::new(Arc::default()),
      has_middleware: AtomicBool::new(false),
      tsr: tsr.unwrap_or(false),
      #[cfg(feature = "plugins")]
      plugins: RwLock::new(Vec::new()),
      #[cfg(feature = "plugins")]
      plugins_initialized: AtomicBool::new(false),
      http_protocol: OnceLock::new(),
      #[cfg(feature = "signals")]
      signals: SignalArbiter::new(),
      #[cfg(any(feature = "utoipa", feature = "vespera"))]
      openapi: RwLock::new(None),
      timeout: OnceLock::new(),
      simd_json_mode: OnceLock::new(),
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

    // RCU-style append: ArcSwap retries the closure on CAS conflict, so
    // concurrent route-level middleware pushes cannot lose entries.
    self.middlewares.rcu(move |current| {
      let mut next = Vec::with_capacity(current.len() + 1);
      next.extend(current.iter().cloned());
      next.push(mw.clone());
      Arc::new(next)
    });
    self.has_middleware.store(true, Ordering::Release);
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
      let plugin_middlewares = mini_router.middlewares.load();
      let existing = self.middlewares.load_full();
      let mut merged = Vec::with_capacity(plugin_middlewares.len() + existing.len());
      merged.extend(plugin_middlewares.iter().cloned());
      merged.extend(existing.iter().cloned());
      if !merged.is_empty() {
        self.has_middleware.store(true, Ordering::Release);
      }
      self.middlewares.store(Arc::new(merged));
    }
  }

  /// Restricts this route to a specific HTTP protocol version.
  ///
  /// Requests whose `version()` does not match are answered with
  /// `505 HTTP Version Not Supported`. Set once at registration; later calls
  /// are no-ops (lock-free reads in the hot path).
  pub fn version(&self, version: http::Version) -> &Self {
    let _ = self.http_protocol.set(version);
    self
  }

  /// HTTP/0.9 guard. Shorthand for [`Route::version`] with [`http::Version::HTTP_09`].
  pub fn h09(&self) -> &Self {
    self.version(http::Version::HTTP_09)
  }

  /// HTTP/1.0 guard. Shorthand for [`Route::version`] with [`http::Version::HTTP_10`].
  pub fn h10(&self) -> &Self {
    self.version(http::Version::HTTP_10)
  }

  /// HTTP/1.1 guard. Shorthand for [`Route::version`] with [`http::Version::HTTP_11`].
  pub fn h11(&self) -> &Self {
    self.version(http::Version::HTTP_11)
  }

  /// HTTP/2 guard. Shorthand for [`Route::version`] with [`http::Version::HTTP_2`].
  pub fn h2(&self) -> &Self {
    self.version(http::Version::HTTP_2)
  }

  /// Returns the configured protocol guard, if any.
  #[inline]
  pub(crate) fn protocol_guard(&self) -> Option<http::Version> {
    self.http_protocol.get().copied()
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

  // OpenAPI metadata methods

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

  /// Sets a timeout for this route, overriding the router-level timeout.
  ///
  /// When a request exceeds the timeout duration, the timeout fallback handler
  /// is invoked (if configured on the router) or a 408 Request Timeout response
  /// is returned.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// use std::time::Duration;
  ///
  /// router.route(Method::POST, "/upload", upload_handler)
  ///     .timeout(Duration::from_secs(60));
  /// ```
  pub fn timeout(&self, duration: Duration) -> &Self {
    let _ = self.timeout.set(duration);
    self
  }

  /// Returns the configured timeout for this route, if any.
  #[inline]
  pub(crate) fn get_timeout(&self) -> Option<Duration> {
    self.timeout.get().copied()
  }

  /// Configures the SIMD JSON dispatch behavior for this route.
  ///
  /// When the `simd` feature is enabled, `Json<T>` can use the `sonic_rs` SIMD
  /// parser for faster deserialization. By default, SIMD is used for payloads
  /// above 2 MB. This method lets you override that threshold — or force
  /// SIMD on/off — for individual routes.
  ///
  /// Without the `simd` feature this setting is accepted but has no effect.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// use tako::extractors::json::SimdJsonMode;
  ///
  /// // Always use SIMD for a heavy ingest endpoint
  /// router.route(Method::POST, "/api/ingest", ingest)
  ///     .simd_json(SimdJsonMode::Always);
  ///
  /// // Use SIMD only above 4 KB
  /// router.route(Method::POST, "/api/batch", batch)
  ///     .simd_json(SimdJsonMode::Threshold(4096));
  ///
  /// // Disable SIMD for a latency-sensitive tiny-payload route
  /// router.route(Method::POST, "/api/ping", ping)
  ///     .simd_json(SimdJsonMode::Never);
  /// ```
  pub fn simd_json(&self, mode: SimdJsonMode) -> &Self {
    let _ = self.simd_json_mode.set(mode);
    self
  }

  /// Returns the configured SIMD JSON mode for this route, if any.
  #[inline]
  pub(crate) fn get_simd_json_mode(&self) -> Option<SimdJsonMode> {
    self.simd_json_mode.get().copied()
  }

  /// Builds a new `Arc<Route>` with the same handler / middlewares / config
  /// but a different path. Used by [`crate::router::Router::nest`] to register
  /// a child router's routes under a prefix without mutating the originals.
  ///
  /// Route-level plugins are *not* carried over — `TakoPlugin` is not `Clone`,
  /// and the cloned route is treated as already-initialized so the empty
  /// plugin list is never set up. Plugin-bearing routes should be registered
  /// directly on the parent router after `nest`.
  pub(crate) fn cloned_with_path(&self, new_path: String) -> Arc<Route> {
    let cloned = Self {
      path: new_path,
      method: self.method.clone(),
      handler: self.handler.clone(),
      middlewares: ArcSwap::new(self.middlewares.load_full()),
      has_middleware: AtomicBool::new(self.has_middleware.load(Ordering::Acquire)),
      tsr: self.tsr,
      #[cfg(feature = "plugins")]
      plugins: RwLock::new(Vec::new()),
      #[cfg(feature = "plugins")]
      plugins_initialized: AtomicBool::new(true),
      http_protocol: {
        let lock = OnceLock::new();
        if let Some(v) = self.http_protocol.get() {
          let _ = lock.set(*v);
        }
        lock
      },
      #[cfg(feature = "signals")]
      signals: SignalArbiter::new(),
      #[cfg(any(feature = "utoipa", feature = "vespera"))]
      openapi: RwLock::new(self.openapi.read().clone()),
      timeout: {
        let lock = OnceLock::new();
        if let Some(v) = self.timeout.get() {
          let _ = lock.set(*v);
        }
        lock
      },
      simd_json_mode: {
        let lock = OnceLock::new();
        if let Some(v) = self.simd_json_mode.get() {
          let _ = lock.set(*v);
        }
        lock
      },
    };
    Arc::new(cloned)
  }
}
