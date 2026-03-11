//! HTTP request routing and dispatch functionality.
//!
//! This module provides the core `Router` struct that manages HTTP routes, middleware chains,
//! and request dispatching. The router supports dynamic path parameters, middleware composition,
//! plugin integration, and global state management. It handles matching incoming requests to
//! registered routes and executing the appropriate handlers through middleware pipelines.
//!
//! # Examples
//!
//! ```rust
//! use tako::{router::Router, Method, responder::Responder, types::Request};
//!
//! async fn hello(_req: Request) -> impl Responder {
//!     "Hello, World!"
//! }
//!
//! async fn user_handler(_req: Request) -> impl Responder {
//!     "User profile"
//! }
//!
//! let mut router = Router::new();
//! router.route(Method::GET, "/", hello);
//! router.route(Method::GET, "/users/{id}", user_handler);
//!
//! // Add global middleware
//! router.middleware(|req, next| async move {
//!     println!("Processing request to: {}", req.uri());
//!     next.run(req).await
//! });
//! ```

use std::sync::Arc;
use std::sync::Weak;
#[cfg(feature = "plugins")]
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use arc_swap::ArcSwap;
use http::Method;
use http::StatusCode;
use scc::HashMap as SccHashMap;
use smallvec::SmallVec;

use crate::body::TakoBody;
use crate::extractors::params::PathParams;
use crate::handler::BoxHandler;
use crate::handler::Handler;
use crate::middleware::Next;
#[cfg(feature = "plugins")]
use crate::plugins::TakoPlugin;
use crate::responder::Responder;
use crate::route::Route;
#[cfg(feature = "signals")]
use crate::signals::Signal;
#[cfg(feature = "signals")]
use crate::signals::SignalArbiter;
#[cfg(feature = "signals")]
use crate::signals::ids;
use crate::state::set_state;
use crate::types::BoxMiddleware;
use crate::types::Request;
use crate::types::Response;

/// HTTP router for managing routes, middleware, and request dispatching.
///
/// The `Router` is the central component for routing HTTP requests to appropriate
/// handlers. It supports dynamic path parameters, middleware chains, plugin integration,
/// and global state management. Routes are matched based on HTTP method and path pattern,
/// with support for trailing slash redirection and parameter extraction.
///
/// # Examples
///
/// ```rust
/// use tako::{router::Router, Method, responder::Responder, types::Request};
///
/// async fn index(_req: Request) -> impl Responder {
///     "Welcome to the home page!"
/// }
///
/// async fn user_profile(_req: Request) -> impl Responder {
///     "User profile page"
/// }
///
/// let mut router = Router::new();
/// router.route(Method::GET, "/", index);
/// router.route(Method::GET, "/users/{id}", user_profile);
/// router.state("app_name", "MyApp".to_string());
/// ```
#[doc(alias = "router")]
/// Type alias for a global error handler function.
///
/// Called when a response has a server error status (5xx). Receives the original
/// response and can transform it (e.g., to return JSON errors instead of plain text).
pub type ErrorHandler = Arc<dyn Fn(Response) -> Response + Send + Sync + 'static>;

pub struct Router {
  /// Map of registered routes keyed by method.
  inner: SccHashMap<Method, matchit::Router<Arc<Route>>>,
  /// An easy-to-iterate index of the same routes so we can access the `Arc<Route>` values
  routes: SccHashMap<Method, Vec<Weak<Route>>>,
  /// Global middleware chain applied to all routes.
  pub(crate) middlewares: ArcSwap<Vec<BoxMiddleware>>,
  /// Optional fallback handler executed when no route matches.
  fallback: Option<BoxHandler>,
  /// Registered plugins for extending functionality.
  #[cfg(feature = "plugins")]
  plugins: Vec<Box<dyn TakoPlugin>>,
  /// Flag to ensure plugins are initialized only once.
  #[cfg(feature = "plugins")]
  plugins_initialized: AtomicBool,
  /// Signal arbiter for in-process event emission and handling.
  #[cfg(feature = "signals")]
  signals: SignalArbiter,
  /// Default timeout for all routes.
  pub(crate) timeout: Option<Duration>,
  /// Fallback handler executed when a request times out.
  timeout_fallback: Option<BoxHandler>,
  /// Global error handler for 5xx responses.
  error_handler: Option<ErrorHandler>,
}

impl Default for Router {
  #[inline]
  fn default() -> Self {
    Self::new()
  }
}

impl Router {
  /// Creates a new, empty router.
  #[must_use]
  pub fn new() -> Self {
    let router = Self {
      inner: SccHashMap::default(),
      routes: SccHashMap::default(),
      middlewares: ArcSwap::new(Arc::default()),
      fallback: None,
      #[cfg(feature = "plugins")]
      plugins: Vec::new(),
      #[cfg(feature = "plugins")]
      plugins_initialized: AtomicBool::new(false),
      #[cfg(feature = "signals")]
      signals: SignalArbiter::new(),
      timeout: None,
      timeout_fallback: None,
      error_handler: None,
    };

    #[cfg(feature = "signals")]
    {
      // If not already present, expose router-level SignalArbiter via global state
      if crate::state::get_state::<SignalArbiter>().is_none() {
        set_state::<SignalArbiter>(router.signals.clone());
      }
    }

    router
  }

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
    let route = Arc::new(Route::new(
      path.to_string(),
      method.clone(),
      BoxHandler::new::<H, T>(handler),
      None,
    ));

    let mut method_router = self.inner.entry_sync(method.clone()).or_default();

    if let Err(err) = method_router
      .get_mut()
      .insert(path.to_string(), route.clone())
    {
      panic!("Failed to register route: {err}");
    }

    self
      .routes
      .entry_sync(method)
      .or_default()
      .push(Arc::downgrade(&route));

    route
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
    if path == "/" {
      panic!("Cannot route with TSR for root path");
    }

    let route = Arc::new(Route::new(
      path.to_string(),
      method.clone(),
      BoxHandler::new::<H, T>(handler),
      Some(true),
    ));

    let mut method_router = self.inner.entry_sync(method.clone()).or_default();

    if let Err(err) = method_router
      .get_mut()
      .insert(path.to_string(), route.clone())
    {
      panic!("Failed to register route: {err}");
    }

    self
      .routes
      .entry_sync(method)
      .or_default()
      .push(Arc::downgrade(&route));

    route
  }

  /// Executes the given endpoint through the global middleware chain.
  ///
  /// This helper is used for cases like TSR redirects and default 404 responses,
  /// ensuring that router-level middleware (e.g., CORS) always runs.
  async fn run_with_global_middlewares_for_endpoint(
    &self,
    req: Request,
    endpoint: BoxHandler,
  ) -> Response {
    // Use load() guard (no Arc refcount bump) for the emptiness check.
    let guard = self.middlewares.load();
    if guard.is_empty() {
      drop(guard);
      endpoint.call(req).await
    } else {
      drop(guard);
      Next {
        global_middlewares: self.middlewares.load_full(),
        route_middlewares: Arc::default(),
        index: 0,
        endpoint,
      }
      .run(req)
      .await
    }
  }

  /// Executes the middleware chain with an optional timeout.
  ///
  /// If a timeout is specified and exceeded, the timeout fallback handler
  /// is invoked or a default 408 Request Timeout response is returned.
  async fn run_with_timeout(
    &self,
    req: Request,
    next: Next,
    timeout_duration: Option<Duration>,
  ) -> Response {
    match timeout_duration {
      Some(duration) => {
        #[cfg(not(feature = "compio"))]
        {
          match tokio::time::timeout(duration, next.run(req)).await {
            Ok(response) => response,
            Err(_elapsed) => self.handle_timeout().await,
          }
        }
        #[cfg(feature = "compio")]
        {
          let sleep = std::pin::pin!(compio::time::sleep(duration));
          let work = std::pin::pin!(next.run(req));
          match futures_util::future::select(work, sleep).await {
            futures_util::future::Either::Left((response, _)) => response,
            futures_util::future::Either::Right((_, _)) => self.handle_timeout().await,
          }
        }
      }
      None => next.run(req).await,
    }
  }

  /// Returns the timeout response using the fallback handler or a default 408.
  async fn handle_timeout(&self) -> Response {
    if let Some(handler) = &self.timeout_fallback {
      handler.call(Request::default()).await
    } else {
      http::Response::builder()
        .status(StatusCode::REQUEST_TIMEOUT)
        .body(TakoBody::empty())
        .expect("valid 408 response")
    }
  }

  /// Dispatches an incoming request to the appropriate route handler.
  pub async fn dispatch(&self, mut req: Request) -> Response {
    let method = req.method().clone();

    // Phase 1: Route lookup using a borrowed path — no String allocation on the
    // hot path. The block scope ensures all borrows on `req` are released before
    // we need to mutate it.
    let route_match = {
      if let Some(method_router) = self.inner.get_sync(&method)
        && let Ok(matched) = method_router.at(req.uri().path())
      {
        let route = Arc::clone(matched.value);
        let mut it = matched.params.iter();
        let first = it.next();
        let params = first.map(|(fk, fv)| {
          let mut p = SmallVec::<[(String, String); 4]>::new();
          p.push((fk.to_string(), fv.to_string()));
          for (k, v) in it {
            p.push((k.to_string(), v.to_string()));
          }
          PathParams(p)
        });
        Some((route, params))
      } else {
        None
      }
    };

    // Phase 2: Dispatch — `req` is no longer borrowed, safe to mutate.
    let response = if let Some((route, params)) = route_match {
      // Protocol guard: early-return if request version does not satisfy route guard
      if let Some(res) = Self::enforce_protocol_guard(&route, &req) {
        return self.maybe_apply_error_handler(res);
      }

      #[cfg(feature = "signals")]
      let route_signals = route.signal_arbiter();

      // Initialize route-level plugins on first request
      #[cfg(feature = "plugins")]
      route.setup_plugins_once();

      // Inject route-level SIMD JSON config into request extensions
      if let Some(mode) = route.get_simd_json_mode() {
        req.extensions_mut().insert(mode);
      }

      if let Some(params) = params {
        req.extensions_mut().insert(params);
      }

      // Determine effective timeout: route-level overrides router-level
      let effective_timeout = route.get_timeout().or(self.timeout);

      // Use load() guards (no Arc refcount bump) to check middleware emptiness.
      // Only call load_full() if we actually need the Arc for the Next chain.
      let g = self.middlewares.load();
      let r = route.middlewares.load();
      let needs_chain = !g.is_empty() || !r.is_empty();
      drop(g);
      drop(r);

      #[cfg(feature = "signals")]
      {
        let method_str = method.to_string();
        let path_str = req.uri().path().to_string();

        route_signals
          .emit(
            Signal::with_capacity(ids::ROUTE_REQUEST_STARTED, 2)
              .meta("method", method_str.clone())
              .meta("path", path_str.clone()),
          )
          .await;

        let response = if !needs_chain && effective_timeout.is_none() {
          route.handler.call(req).await
        } else {
          let next = Next {
            global_middlewares: self.middlewares.load_full(),
            route_middlewares: route.middlewares.load_full(),
            index: 0,
            endpoint: route.handler.clone(),
          };
          self.run_with_timeout(req, next, effective_timeout).await
        };

        route_signals
          .emit(
            Signal::with_capacity(ids::ROUTE_REQUEST_COMPLETED, 3)
              .meta("method", method_str)
              .meta("path", path_str)
              .meta("status", response.status().as_u16().to_string()),
          )
          .await;

        response
      }

      #[cfg(not(feature = "signals"))]
      {
        if !needs_chain && effective_timeout.is_none() {
          route.handler.call(req).await
        } else {
          let next = Next {
            global_middlewares: self.middlewares.load_full(),
            route_middlewares: route.middlewares.load_full(),
            index: 0,
            endpoint: route.handler.clone(),
          };
          self.run_with_timeout(req, next, effective_timeout).await
        }
      }
    } else {
      // Cold path: no direct match — try TSR redirect / fallback.
      // String allocation is acceptable here.
      let tsr_path = {
        let p = req.uri().path();
        if p.ends_with('/') {
          p.trim_end_matches('/').to_string()
        } else {
          format!("{p}/")
        }
      };

      if let Some(method_router) = self.inner.get_sync(&method)
        && let Ok(matched) = method_router.at(&tsr_path)
        && matched.value.tsr
      {
        let handler = move |_req: Request| async move {
          http::Response::builder()
            .status(StatusCode::TEMPORARY_REDIRECT)
            .header("Location", tsr_path.clone())
            .body(TakoBody::empty())
            .expect("valid redirect response")
        };

        self
          .run_with_global_middlewares_for_endpoint(req, BoxHandler::new::<_, (Request,)>(handler))
          .await
      } else if let Some(handler) = &self.fallback {
        self
          .run_with_global_middlewares_for_endpoint(req, handler.clone())
          .await
      } else {
        let handler = |_req: Request| async {
          http::Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(TakoBody::empty())
            .expect("valid 404 response")
        };

        self
          .run_with_global_middlewares_for_endpoint(req, BoxHandler::new::<_, (Request,)>(handler))
          .await
      }
    };

    self.maybe_apply_error_handler(response)
  }

  /// Applies the global error handler if one is set and the response is a server error.
  fn maybe_apply_error_handler(&self, response: Response) -> Response {
    if response.status().is_server_error() {
      if let Some(handler) = &self.error_handler {
        return handler(response);
      }
    }
    response
  }

  /// Adds a value to the global type-based state accessible by all handlers.
  ///
  /// Global state allows sharing data across different routes and middleware.
  /// Values are stored by their concrete type and retrieved via the
  /// [`State`](crate::extractors::state::State) extractor or with
  /// [`crate::state::get_state`].
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::router::Router;
  ///
  /// #[derive(Clone)]
  /// struct AppConfig { database_url: String, api_key: String }
  ///
  /// let mut router = Router::new();
  /// router.state(AppConfig {
  ///     database_url: "postgresql://localhost/mydb".to_string(),
  ///     api_key: "secret-key".to_string(),
  /// });
  /// // You can also store simple types by type:
  /// router.state::<String>("1.0.0".to_string());
  /// ```
  pub fn state<T: Clone + Send + Sync + 'static>(&mut self, value: T) {
    set_state(value);
  }

  #[cfg(feature = "signals")]
  /// Returns a reference to the signal arbiter.
  pub fn signals(&self) -> &SignalArbiter {
    &self.signals
  }

  #[cfg(feature = "signals")]
  /// Returns a clone of the signal arbiter, useful for sharing through state.
  pub fn signal_arbiter(&self) -> SignalArbiter {
    self.signals.clone()
  }

  #[cfg(feature = "signals")]
  /// Registers a handler for a named signal on this router's arbiter.
  pub fn on_signal<F, Fut>(&self, id: impl Into<String>, handler: F)
  where
    F: Fn(Signal) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
  {
    self.signals.on(id, handler);
  }

  #[cfg(feature = "signals")]
  /// Emits a signal through this router's arbiter.
  pub async fn emit_signal(&self, signal: Signal) {
    self.signals.emit(signal).await;
  }

  /// Adds global middleware to the router.
  ///
  /// Global middleware is executed for all routes in the order it was added,
  /// before any route-specific middleware. Middleware can modify requests,
  /// generate responses, or perform side effects like logging or authentication.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::{router::Router, middleware::Next, types::Request};
  ///
  /// let mut router = Router::new();
  ///
  /// // Logging middleware
  /// router.middleware(|req, next| async move {
  ///     println!("Request: {} {}", req.method(), req.uri());
  ///     let response = next.run(req).await;
  ///     println!("Response: {}", response.status());
  ///     response
  /// });
  ///
  /// // Authentication middleware
  /// router.middleware(|req, next| async move {
  ///     if req.headers().contains_key("authorization") {
  ///         next.run(req).await
  ///     } else {
  ///         "Unauthorized".into_response()
  ///     }
  /// });
  /// ```
  pub fn middleware<F, Fut, R>(&self, f: F) -> &Self
  where
    F: Fn(Request, Next) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = R> + Send + 'static,
    R: Responder + Send + 'static,
  {
    let mw: BoxMiddleware = Arc::new(move |req, next| {
      let fut = f(req, next);
      Box::pin(async move { fut.await.into_response() })
    });

    let mut middlewares = self.middlewares.load().iter().cloned().collect::<Vec<_>>();
    middlewares.push(mw);
    self.middlewares.store(Arc::new(middlewares));
    self
  }

  /// Sets a fallback handler that will be executed when no route matches.
  ///
  /// The fallback runs after global middlewares and can be used to implement
  /// custom 404 pages, catch-all logic, or method-independent handlers.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::{router::Router, Method, responder::Responder, types::Request};
  ///
  /// async fn not_found(_req: Request) -> impl Responder { "Not Found" }
  ///
  /// let mut router = Router::new();
  /// router.route(Method::GET, "/", |_req| async { "Hello" });
  /// router.fallback(not_found);
  /// ```
  pub fn fallback<F, Fut, R>(&mut self, handler: F) -> &mut Self
  where
    F: Fn(Request) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = R> + Send + 'static,
    R: Responder + Send + 'static,
  {
    // Use the Request-arg handler impl to box the fallback
    self.fallback = Some(BoxHandler::new::<F, (Request,)>(handler));
    self
  }

  /// Sets a fallback handler that supports extractors (like `Path`, `Query`, etc.).
  ///
  /// Use this when your fallback needs to parse request data via extractors. If you
  /// only need access to the raw `Request`, prefer `fallback` for simpler type inference.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::{router::Router, responder::Responder, extractors::{path::Path, query::Query}};
  ///
  /// #[derive(serde::Deserialize)]
  /// struct Q { q: Option<String> }
  ///
  /// async fn fallback_with_q(Path(_p): Path<String>, Query(_q): Query<Q>) -> impl Responder {
  ///     "Not Found"
  /// }
  ///
  /// let mut router = Router::new();
  /// router.fallback_with_extractors(fallback_with_q);
  /// ```
  pub fn fallback_with_extractors<H, T>(&mut self, handler: H) -> &mut Self
  where
    H: Handler<T> + Clone + 'static,
  {
    self.fallback = Some(BoxHandler::new::<H, T>(handler));
    self
  }

  /// Sets a default timeout for all routes.
  ///
  /// This timeout can be overridden on individual routes using `Route::timeout`.
  /// When a request exceeds the timeout duration, the timeout fallback handler
  /// is invoked (if configured) or a 408 Request Timeout response is returned.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::router::Router;
  /// use std::time::Duration;
  ///
  /// let mut router = Router::new();
  /// router.timeout(Duration::from_secs(30));
  /// ```
  pub fn timeout(&mut self, duration: Duration) -> &mut Self {
    self.timeout = Some(duration);
    self
  }

  /// Sets a fallback handler that will be executed when a request times out.
  ///
  /// If no timeout fallback is set, a default 408 Request Timeout response is returned.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::{router::Router, responder::Responder, types::Request};
  /// use std::time::Duration;
  ///
  /// async fn timeout_handler(_req: Request) -> impl Responder {
  ///     "Request took too long"
  /// }
  ///
  /// let mut router = Router::new();
  /// router.timeout(Duration::from_secs(30));
  /// router.timeout_fallback(timeout_handler);
  /// ```
  pub fn timeout_fallback<F, Fut, R>(&mut self, handler: F) -> &mut Self
  where
    F: Fn(Request) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = R> + Send + 'static,
    R: Responder + Send + 'static,
  {
    self.timeout_fallback = Some(BoxHandler::new::<F, (Request,)>(handler));
    self
  }

  /// Sets a global error handler for 5xx responses.
  ///
  /// The error handler receives any response with a server error status and can
  /// transform it (e.g., to return JSON-formatted errors instead of plain text).
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::router::Router;
  /// use tako::body::TakoBody;
  ///
  /// let mut router = Router::new();
  /// router.error_handler(|resp| {
  ///     let status = resp.status();
  ///     let body = format!(r#"{{"error": "{}"}}"#, status.canonical_reason().unwrap_or("Unknown"));
  ///     let mut res = http::Response::new(TakoBody::from(body));
  ///     *res.status_mut() = status;
  ///     res.headers_mut().insert(
  ///         http::header::CONTENT_TYPE,
  ///         http::HeaderValue::from_static("application/json"),
  ///     );
  ///     res
  /// });
  /// ```
  pub fn error_handler(
    &mut self,
    handler: impl Fn(Response) -> Response + Send + Sync + 'static,
  ) -> &mut Self {
    self.error_handler = Some(Arc::new(handler));
    self
  }

  /// Registers a plugin with the router.
  ///
  /// Plugins extend the router's functionality by providing additional features
  /// like compression, CORS handling, rate limiting, or custom behavior. Plugins
  /// are initialized once when the server starts.
  ///
  /// # Examples
  ///
  /// ```rust
  /// # #[cfg(feature = "plugins")]
  /// use tako::{router::Router, plugins::TakoPlugin};
  /// # #[cfg(feature = "plugins")]
  /// use anyhow::Result;
  ///
  /// # #[cfg(feature = "plugins")]
  /// struct LoggingPlugin;
  ///
  /// # #[cfg(feature = "plugins")]
  /// impl TakoPlugin for LoggingPlugin {
  ///     fn name(&self) -> &'static str {
  ///         "logging"
  ///     }
  ///
  ///     fn setup(&self, _router: &Router) -> Result<()> {
  ///         println!("Logging plugin initialized");
  ///         Ok(())
  ///     }
  /// }
  ///
  /// # #[cfg(feature = "plugins")]
  /// # fn example() {
  /// let mut router = Router::new();
  /// router.plugin(LoggingPlugin);
  /// # }
  /// ```
  #[cfg(feature = "plugins")]
  #[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
  pub fn plugin<P>(&mut self, plugin: P) -> &mut Self
  where
    P: TakoPlugin + Clone + Send + Sync + 'static,
  {
    self.plugins.push(Box::new(plugin));
    self
  }

  /// Returns references to all registered plugins.
  #[cfg(feature = "plugins")]
  #[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
  pub(crate) fn plugins(&self) -> Vec<&dyn TakoPlugin> {
    self.plugins.iter().map(|plugin| plugin.as_ref()).collect()
  }

  /// Initializes all registered plugins exactly once.
  #[cfg(feature = "plugins")]
  #[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
  pub(crate) fn setup_plugins_once(&self) {
    use std::sync::atomic::Ordering;

    if !self.plugins_initialized.swap(true, Ordering::SeqCst) {
      for plugin in self.plugins() {
        let _ = plugin.setup(self);
      }
    }
  }

  /// Merges another router into this router.
  ///
  /// This method combines routes and middleware from another router into the
  /// current one. Routes are copied over, and the other router's global middleware
  /// is prepended to each merged route's middleware chain.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::{router::Router, Method, responder::Responder, types::Request};
  ///
  /// async fn api_handler(_req: Request) -> impl Responder {
  ///     "API response"
  /// }
  ///
  /// async fn web_handler(_req: Request) -> impl Responder {
  ///     "Web response"
  /// }
  ///
  /// // Create API router
  /// let mut api_router = Router::new();
  /// api_router.route(Method::GET, "/users", api_handler);
  /// api_router.middleware(|req, next| async move {
  ///     println!("API middleware");
  ///     next.run(req).await
  /// });
  ///
  /// // Create main router and merge API router
  /// let mut main_router = Router::new();
  /// main_router.route(Method::GET, "/", web_handler);
  /// main_router.merge(api_router);
  /// ```
  pub fn merge(&mut self, other: Router) {
    let upstream_globals = other.middlewares.load_full();

    other.routes.iter_sync(|method, weak_vec| {
      let mut target_router = self.inner.entry_sync(method.clone()).or_default();

      for weak in weak_vec {
        if let Some(route) = weak.upgrade() {
          let existing = route.middlewares.load_full();
          let mut merged = Vec::with_capacity(upstream_globals.len() + existing.len());
          merged.extend(upstream_globals.iter().cloned());
          merged.extend(existing.iter().cloned());
          route.middlewares.store(Arc::new(merged));

          let _ = target_router
            .get_mut()
            .insert(route.path.clone(), route.clone());

          self
            .routes
            .entry_sync(method.clone())
            .or_default()
            .push(Arc::downgrade(&route));
        }
      }

      true
    });

    #[cfg(feature = "signals")]
    self.signals.merge_from(&other.signals);
  }

  /// Ensures the request HTTP version satisfies the route's configured protocol guard.
  /// Returns `Some(Response)` with 505 HTTP Version Not Supported when the request
  /// doesn't match the guard, otherwise returns `None` to continue dispatch.
  fn enforce_protocol_guard(route: &Route, req: &Request) -> Option<Response> {
    if let Some(guard) = route.protocol_guard()
      && guard != req.version()
    {
      return Some(
        http::Response::builder()
          .status(StatusCode::HTTP_VERSION_NOT_SUPPORTED)
          .body(TakoBody::empty())
          .expect("valid HTTP version not supported response"),
      );
    }
    None
  }

  // OpenAPI route collection

  /// Collects OpenAPI metadata from all registered routes.
  ///
  /// Returns a vector of tuples containing the HTTP method, path, and OpenAPI
  /// metadata for each route that has OpenAPI information attached.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// use tako::{router::Router, Method};
  ///
  /// let mut router = Router::new();
  /// router.route(Method::GET, "/users", list_users)
  ///     .summary("List users")
  ///     .tag("users");
  ///
  /// for (method, path, openapi) in router.collect_openapi_routes() {
  ///     println!("{} {} - {:?}", method, path, openapi.summary);
  /// }
  /// ```
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn collect_openapi_routes(&self) -> Vec<(Method, String, crate::openapi::RouteOpenApi)> {
    let mut result = Vec::new();

    self.routes.iter_sync(|method, weak_vec| {
      for weak in weak_vec {
        if let Some(route) = weak.upgrade() {
          if let Some(openapi) = route.openapi_metadata() {
            result.push((method.clone(), route.path.clone(), openapi));
          }
        }
      }
      true
    });

    result
  }
}
