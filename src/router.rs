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

use std::{
  collections::HashMap,
  sync::{Arc, Weak},
};

use dashmap::DashMap;
use http::Method;
use http::StatusCode;
use parking_lot::RwLock;

use crate::{
  body::TakoBody,
  extractors::params::PathParams,
  handler::{BoxHandler, Handler},
  middleware::Next,
  responder::Responder,
  route::Route,
  state::set_state,
  types::{BoxMiddleware, Request, Response},
};

#[cfg(feature = "signals")]
use crate::signals::{Signal, SignalArbiter, ids};

#[cfg(feature = "plugins")]
use crate::plugins::TakoPlugin;

#[cfg(feature = "plugins")]
use std::sync::atomic::AtomicBool;

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
pub struct Router {
  /// Map of registered routes keyed by method.
  inner: DashMap<Method, matchit::Router<Arc<Route>>>,
  /// An easy-to-iterate index of the same routes so we can access the `Arc<Route>` values
  routes: DashMap<Method, Vec<Weak<Route>>>,
  /// Global middleware chain applied to all routes.
  pub(crate) middlewares: RwLock<Vec<BoxMiddleware>>,
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
}

impl Router {
  /// Creates a new, empty router.
  pub fn new() -> Self {
    Self {
      inner: DashMap::default(),
      routes: DashMap::default(),
      middlewares: RwLock::new(Vec::new()),
      fallback: None,
      #[cfg(feature = "plugins")]
      plugins: Vec::new(),
      #[cfg(feature = "plugins")]
      plugins_initialized: AtomicBool::new(false),
      #[cfg(feature = "signals")]
      signals: SignalArbiter::new(),
    }
  }

  /// Registers a new route with the router.
  ///
  /// Associates an HTTP method and path pattern with a handler function. The path
  /// can contain dynamic segments using curly braces (e.g., `/users/{id}`), which
  /// are extracted as parameters during request processing.
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

    let mut method_router = self.inner.entry(method.clone()).or_default();

    if let Err(err) = method_router.insert(path.to_string(), route.clone()) {
      panic!("Failed to register route: {err}");
    }

    self
      .routes
      .entry(method)
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
  /// Panics if called with the root path ("/") since TSR is not applicable.
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

    let mut method_router = self.inner.entry(method.clone()).or_default();

    if let Err(err) = method_router.insert(path.to_string(), route.clone()) {
      panic!("Failed to register route: {err}");
    }

    self
      .routes
      .entry(method)
      .or_default()
      .push(Arc::downgrade(&route));

    route
  }

  /// Dispatches an incoming request to the appropriate route handler.
  pub async fn dispatch(&self, mut req: Request) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    if let Some(method_router) = self.inner.get(&method)
      && let Ok(matched) = method_router.at(&path)
    {
      let route = matched.value;

      // Protocol guard: early-return if request version does not satisfy route guard
      if let Some(res) = Self::enforce_protocol_guard(route, &req) {
        return res;
      }

      #[cfg(feature = "signals")]
      let route_signals = route.signal_arbiter();

      // Initialize route-level plugins on first request
      #[cfg(feature = "plugins")]
      route.setup_plugins_once();

      if !matched.params.iter().collect::<Vec<_>>().is_empty() {
        let mut params = HashMap::new();
        for (k, v) in matched.params.iter() {
          params.insert(k.to_string(), v.to_string());
        }
        req.extensions_mut().insert(PathParams(params));
      }
      let g_mws = self.middlewares.read().clone();
      let r_mws = route.middlewares.read().clone();
      let mut chain = Vec::new();
      chain.extend(g_mws.into_iter());
      chain.extend(r_mws.into_iter());

      let next = Next {
        middlewares: Arc::new(chain),
        endpoint: Arc::new(route.handler.clone()),
      };

      #[cfg(feature = "signals")]
      {
        let method_str = method.to_string();
        let path_str = path.clone();

        let mut start_meta = HashMap::new();
        start_meta.insert("method".to_string(), method_str.clone());
        start_meta.insert("path".to_string(), path_str.clone());
        route_signals
          .emit(Signal::with_metadata(
            ids::ROUTE_REQUEST_STARTED,
            start_meta,
          ))
          .await;

        let response = next.run(req).await;

        let mut done_meta = HashMap::new();
        done_meta.insert("method".to_string(), method_str);
        done_meta.insert("path".to_string(), path_str);
        done_meta.insert("status".to_string(), response.status().as_u16().to_string());
        route_signals
          .emit(Signal::with_metadata(
            ids::ROUTE_REQUEST_COMPLETED,
            done_meta,
          ))
          .await;

        return response;
      }

      #[cfg(not(feature = "signals"))]
      {
        return next.run(req).await;
      }
    }

    let tsr_path = if path.ends_with('/') {
      path.trim_end_matches('/').to_string()
    } else {
      format!("{path}/")
    };

    if let Some(method_router) = self.inner.get(&method)
      && let Ok(matched) = method_router.at(&tsr_path)
      && matched.value.tsr
    {
      return http::Response::builder()
        .status(StatusCode::TEMPORARY_REDIRECT)
        .header("Location", tsr_path)
        .body(TakoBody::empty())
        .unwrap();
    }

    // No match: use fallback handler if configured
    if let Some(handler) = &self.fallback {
      let g_mws = self.middlewares.read().clone();
      let next = Next {
        middlewares: Arc::new(g_mws),
        endpoint: Arc::new(handler.clone()),
      };
      return next.run(req).await;
    }

    http::Response::builder()
      .status(StatusCode::NOT_FOUND)
      .body(TakoBody::empty())
      .unwrap()
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

    self.middlewares.write().push(mw);
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
    let upstream_globals = other.middlewares.read().clone();

    for (method, weak_vec) in other.routes.into_iter() {
      let mut target_router = self.inner.entry(method.clone()).or_default();

      for weak in weak_vec {
        if let Some(route) = weak.upgrade() {
          let mut rmw = route.middlewares.write();
          for mw in upstream_globals.iter().rev() {
            rmw.push_front(mw.clone());
          }

          let _ = target_router.insert(route.path.clone(), route.clone());

          self
            .routes
            .entry(method.clone())
            .or_default()
            .push(Arc::downgrade(&route));
        }
      }
    }

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
          .unwrap(),
      );
    }
    None
  }
}
