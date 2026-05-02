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
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use arc_swap::ArcSwap;
use http::Method;
use http::StatusCode;
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
use crate::router_state::RouterState;
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
  /// Map of registered routes keyed by method (O(1) array lookup).
  inner: MethodMap<matchit::Router<Arc<Route>>>,
  /// An easy-to-iterate index of the same routes so we can access the `Arc<Route>` values.
  routes: MethodMap<Vec<Weak<Route>>>,
  /// Optional path prefix prepended to every `route()` call while it is set.
  /// Used by [`Router::mount_all_into`] and [`Router::scope`] (see v2 roadmap).
  /// Only consulted at registration time — zero cost on the dispatch hot path.
  pending_prefix: Option<String>,
  /// Global middleware chain applied to all routes.
  pub(crate) middlewares: ArcSwap<Vec<BoxMiddleware>>,
  /// Fast check: true when global middleware is registered (avoids ArcSwap load on hot path).
  has_global_middleware: AtomicBool,
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
  /// Global error handler for 4xx responses (opt-in; runs after dispatch).
  client_error_handler: Option<ErrorHandler>,
  /// Per-router typed state populated via [`Router::with_state`].
  /// `Arc` is shared with every dispatched request via the request extension
  /// so the `State<T>` extractor can read instance-local values.
  router_state: Arc<RouterState>,
  /// Fast-path flag: when `false`, dispatch skips the per-request Arc clone +
  /// extension insert that wires `router_state` into requests.
  has_router_state: AtomicBool,
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
      inner: MethodMap::new(),
      routes: MethodMap::new(),
      pending_prefix: None,
      middlewares: ArcSwap::new(Arc::default()),
      has_global_middleware: AtomicBool::new(false),
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
      client_error_handler: None,
      router_state: Arc::new(RouterState::new()),
      has_router_state: AtomicBool::new(false),
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
  fn apply_pending_prefix(&self, path: &str) -> String {
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

  /// Registers every route declared via the `#[tako::route]` / `#[tako::get]`
  /// (and friends) attribute macros into this router.
  ///
  /// Each macro contributes a thunk into the global [`TAKO_ROUTES`] slice at
  /// link time; this method walks the slice and invokes each thunk against
  /// `self`, which calls [`Router::route`] under the hood. Routes are
  /// registered in the order the linker emits them — typically the order they
  /// appear within a translation unit, but unspecified across crates. If two
  /// thunks register the same `(method, path)` pair, the second call will
  /// panic, matching the behavior of [`Router::route`].
  ///
  /// # Examples
  ///
  /// ```ignore
  /// use tako::{get, router::Router};
  ///
  /// #[get("/health")]
  /// async fn health() -> impl tako::responder::Responder { "ok" }
  ///
  /// let mut router = Router::new();
  /// router.mount_all();
  /// ```
  pub fn mount_all(&mut self) -> &mut Self {
    for register in TAKO_ROUTES {
      register(self);
    }
    self
  }

  /// Like [`Router::mount_all`] but registers every macro-declared route under
  /// the given path prefix. The prefix is normalized (trailing `/` stripped),
  /// then prepended to each registered path. Useful when you want, e.g., all
  /// `#[get("/users")]` declarations to live under `/api`.
  ///
  /// Ordering across crates remains the linker's choice (see
  /// [`Router::mount_all`] for details).
  ///
  /// # Examples
  ///
  /// ```ignore
  /// let mut router = Router::new();
  /// router.mount_all_into("/api"); // /users → /api/users, /health → /api/health
  /// ```
  pub fn mount_all_into(&mut self, prefix: &str) -> &mut Self {
    let saved = self.pending_prefix.take();
    self.pending_prefix = Some(prefix.to_string());
    for register in TAKO_ROUTES {
      register(self);
    }
    self.pending_prefix = saved;
    self
  }

  /// Registers a group of routes under a shared path prefix.
  ///
  /// The closure receives `self` with the prefix active, so any `route()` /
  /// `get()` / `post()` etc. calls inside register the routes with the prefix
  /// prepended. Prefixes nest: a `scope("/v1", |r| r.scope("/users", …))`
  /// produces routes under `/v1/users`. Cold path; no dispatch impact.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::router::Router;
  /// use tako::responder::Responder;
  ///
  /// async fn list_users() -> impl Responder { "users" }
  /// async fn create_user() -> impl Responder { "created" }
  ///
  /// let mut router = Router::new();
  /// router.scope("/api/v1", |r| {
  ///     r.get("/users", list_users);
  ///     r.post("/users", create_user);
  /// });
  /// ```
  pub fn scope<F>(&mut self, prefix: &str, build: F) -> &mut Self
  where
    F: FnOnce(&mut Router),
  {
    let saved = self.pending_prefix.take();
    let new_prefix = match &saved {
      Some(parent) => {
        let parent = parent.trim_end_matches('/');
        if prefix.starts_with('/') {
          format!("{parent}{prefix}")
        } else {
          format!("{parent}/{prefix}")
        }
      }
      None => prefix.to_string(),
    };
    self.pending_prefix = Some(new_prefix);
    build(self);
    self.pending_prefix = saved;
    self
  }

  /// Mounts every route from a child router under the given path prefix.
  ///
  /// Unlike [`Router::merge`], `nest` builds **new** `Arc<Route>` instances for
  /// each child route via `Route::cloned_with_path` — so re-nesting the same
  /// child cannot double-stack its global middleware onto the same shared
  /// `Arc<Route>`. The child router's global middleware chain is prepended to
  /// each newly-registered route's middleware chain (so child globals run
  /// before child-route middleware at dispatch time).
  ///
  /// Caveats:
  /// - Route-level plugins on the child are **not** carried over.
  /// - The child's fallback / error handlers are **not** inherited.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::router::Router;
  /// use tako::responder::Responder;
  ///
  /// async fn list_users() -> impl Responder { "users" }
  ///
  /// let mut api = Router::new();
  /// api.get("/users", list_users);
  ///
  /// let mut root = Router::new();
  /// root.nest("/api/v1", api); // /users → /api/v1/users
  /// ```
  pub fn nest(&mut self, prefix: &str, child: Router) -> &mut Self {
    let upstream_globals = child.middlewares.load_full();

    for (method, weak_vec) in child.routes.iter() {
      for weak in weak_vec {
        let Some(child_route) = weak.upgrade() else {
          continue;
        };

        let combined = combine_prefix_path(prefix, &child_route.path);
        let new_path = self.apply_pending_prefix(&combined);

        let new_route = child_route.cloned_with_path(new_path.clone());

        if !upstream_globals.is_empty() {
          let existing = new_route.middlewares.load_full();
          let mut merged = Vec::with_capacity(upstream_globals.len() + existing.len());
          merged.extend(upstream_globals.iter().cloned());
          merged.extend(existing.iter().cloned());
          new_route.has_middleware.store(true, Ordering::Release);
          new_route.middlewares.store(Arc::new(merged));
        }

        if let Err(err) = self
          .inner
          .get_or_default_mut(&method)
          .insert(new_path, new_route.clone())
        {
          panic!("Failed to nest route: {err}");
        }
        self
          .routes
          .get_or_default_mut(&method)
          .push(Arc::downgrade(&new_route));
      }
    }

    #[cfg(feature = "signals")]
    self.signals.merge_from(&child.signals);

    self
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

  /// Executes the given endpoint through the global middleware chain.
  ///
  /// This helper is used for cases like TSR redirects and default 404 responses,
  /// ensuring that router-level middleware (e.g., CORS) always runs.
  async fn run_with_global_middlewares_for_endpoint(
    &self,
    req: Request,
    endpoint: BoxHandler,
  ) -> Response {
    if !self.has_global_middleware.load(Ordering::Acquire) {
      endpoint.call(req).await
    } else {
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
  #[inline]
  pub async fn dispatch(&self, mut req: Request) -> Response {
    // Per-router state: only inject when at least one `with_state` was called.
    // The atomic load is monomorphic and cheap; the Arc clone (atomic incref)
    // only happens for routers that actually use instance-local state.
    if self.has_router_state.load(Ordering::Acquire) {
      req.extensions_mut().insert(Arc::clone(&self.router_state));
    }

    // App-level request signal — emitted here so every transport gets it for
    // free without duplicating the boilerplate. The cost is a single string
    // formatting pair per request and is gated to the `signals` feature.
    #[cfg(feature = "signals")]
    let (req_method_str, req_path_str) = (req.method().to_string(), req.uri().path().to_string());
    #[cfg(feature = "signals")]
    {
      SignalArbiter::emit_app(
        Signal::with_capacity(ids::REQUEST_STARTED, 2)
          .meta("method", req_method_str.clone())
          .meta("path", req_path_str.clone()),
      )
      .await;
    }

    // Phase 1: Route lookup using a borrowed path — no String allocation on the
    // hot path. The block scope ensures all borrows on `req` are released before
    // we need to mutate it.
    let route_match = {
      if let Some(method_router) = self.inner.get(req.method())
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

      // Inject the matched route template (e.g. `/users/{id}`) so handlers
      // and middleware can label metrics/logs by the routing key, not the
      // concrete URI.
      req
        .extensions_mut()
        .insert(crate::router_state::MatchedPath(route.path.clone()));

      // Determine effective timeout: route-level overrides router-level
      let effective_timeout = route.get_timeout().or(self.timeout);

      // Fast atomic check: skip ArcSwap loads entirely when no middleware is registered.
      let needs_chain = self.has_global_middleware.load(Ordering::Acquire)
        || route.has_middleware.load(Ordering::Acquire);

      #[cfg(feature = "signals")]
      {
        let method_str = req.method().to_string();
        let path_str = req.uri().path().to_string();
        let route_template = route.path.clone();

        route_signals
          .emit(
            Signal::with_capacity(ids::ROUTE_REQUEST_STARTED, 3)
              .meta("method", method_str.clone())
              .meta("path", path_str.clone())
              .meta("route", route_template.clone()),
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
            Signal::with_capacity(ids::ROUTE_REQUEST_COMPLETED, 4)
              .meta("method", method_str)
              .meta("path", path_str)
              .meta("route", route_template)
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
      // Cold path: no direct match — try TSR redirect / 405 / fallback.
      // String allocation is acceptable here.
      let tsr_path = {
        let p = req.uri().path();
        if p.ends_with('/') {
          p.trim_end_matches('/').to_string()
        } else {
          format!("{p}/")
        }
      };

      if let Some(method_router) = self.inner.get(req.method())
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
      } else {
        // Method-mismatch detection: if the same path is registered for any
        // *other* method, RFC 9110 mandates 405 with an `Allow` header rather
        // than 404. This is the cold path; iterating the 9 standard methods
        // is cheap.
        let allowed = self.collect_allowed_methods(req.uri().path());
        if !allowed.is_empty() {
          let allow_value = join_methods(&allowed);
          let handler = move |_req: Request| async move {
            http::Response::builder()
              .status(StatusCode::METHOD_NOT_ALLOWED)
              .header(http::header::ALLOW, allow_value.clone())
              .body(TakoBody::empty())
              .expect("valid 405 response")
          };
          self
            .run_with_global_middlewares_for_endpoint(
              req,
              BoxHandler::new::<_, (Request,)>(handler),
            )
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
            .run_with_global_middlewares_for_endpoint(
              req,
              BoxHandler::new::<_, (Request,)>(handler),
            )
            .await
        }
      }
    };

    let response = self.maybe_apply_error_handler(response);

    #[cfg(feature = "signals")]
    {
      SignalArbiter::emit_app(
        Signal::with_capacity(ids::REQUEST_COMPLETED, 3)
          .meta("method", req_method_str)
          .meta("path", req_path_str)
          .meta("status", response.status().as_u16().to_string()),
      )
      .await;
    }

    response
  }

  /// Applies the appropriate error handler if one is set:
  /// - 5xx → [`Router::error_handler`]
  /// - 4xx → [`Router::client_error_handler`]
  fn maybe_apply_error_handler(&self, response: Response) -> Response {
    let status = response.status();
    if status.is_server_error() {
      if let Some(handler) = &self.error_handler {
        return handler(response);
      }
    } else if status.is_client_error() {
      if let Some(handler) = &self.client_error_handler {
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

  /// Inserts a value into this router's instance-local typed state.
  ///
  /// Unlike [`Router::state`] (which writes the process-global registry and
  /// therefore allows only one value per `T` per process), `with_state` is
  /// per-router — multiple routers can hold distinct `T`s without collisions.
  ///
  /// The [`crate::extractors::state::State`] extractor reads the per-router
  /// store first and falls back to the global store if no per-router value
  /// exists, so existing code that uses `set_state` / `Router::state`
  /// continues to work unchanged.
  ///
  /// Hot-path cost is one `Arc` clone per request *only when* at least one
  /// `with_state` call has happened on this router; an `AtomicBool::Acquire`
  /// fast-path skips it for routers that don't use instance-local state.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::router::Router;
  ///
  /// #[derive(Clone)]
  /// struct Db;
  ///
  /// let mut router = Router::new();
  /// router.with_state(Db);
  /// ```
  pub fn with_state<T: Clone + Send + Sync + 'static>(&mut self, value: T) -> &mut Self {
    self.router_state.insert(value);
    self.has_router_state.store(true, Ordering::Release);
    self
  }

  /// Returns the per-router typed state (shared `Arc`).
  #[inline]
  pub fn router_state(&self) -> &Arc<RouterState> {
    &self.router_state
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

    // RCU-style append: rebuild the Vec atomically against concurrent pushers.
    // ArcSwap retries the closure on CAS conflict, so concurrent middleware
    // registrations cannot lose entries.
    self.middlewares.rcu(move |current| {
      let mut next = Vec::with_capacity(current.len() + 1);
      next.extend(current.iter().cloned());
      next.push(mw.clone());
      Arc::new(next)
    });
    self.has_global_middleware.store(true, Ordering::Release);
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

  /// Sets a global error handler for 4xx responses.
  ///
  /// Mirrors [`Router::error_handler`] but fires for client errors. Useful for
  /// converting bare 404 / 405 / 422 responses into structured error documents
  /// (e.g. via [`crate::problem::default_problem_responder`]).
  pub fn client_error_handler(
    &mut self,
    handler: impl Fn(Response) -> Response + Send + Sync + 'static,
  ) -> &mut Self {
    self.client_error_handler = Some(Arc::new(handler));
    self
  }

  /// Convenience: install [`crate::problem::default_problem_responder`] for
  /// both 4xx and 5xx so unhandled errors always render as
  /// `application/problem+json`.
  pub fn use_problem_json(&mut self) -> &mut Self {
    let h: ErrorHandler = Arc::new(crate::problem::default_problem_responder);
    self.error_handler = Some(h.clone());
    self.client_error_handler = Some(h);
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
  #[doc(hidden)]
  pub fn setup_plugins_once(&self) {
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

    for (method, weak_vec) in other.routes.iter() {
      for weak in weak_vec {
        if let Some(route) = weak.upgrade() {
          let existing = route.middlewares.load_full();
          let mut merged = Vec::with_capacity(upstream_globals.len() + existing.len());
          merged.extend(upstream_globals.iter().cloned());
          merged.extend(existing.iter().cloned());
          if !merged.is_empty() {
            route.has_middleware.store(true, Ordering::Release);
          }
          route.middlewares.store(Arc::new(merged));

          let _ = self
            .inner
            .get_or_default_mut(&method)
            .insert(route.path.clone(), route.clone());

          self
            .routes
            .get_or_default_mut(&method)
            .push(Arc::downgrade(&route));
        }
      }
    }

    #[cfg(feature = "signals")]
    self.signals.merge_from(&other.signals);
  }

  /// Returns every method that has a route matching the given path.
  ///
  /// Used by the 405 / `Allow` cold-path branch in [`Router::dispatch`]; not on
  /// the fast path. Iterates all standard methods (O(9)) plus any custom ones.
  fn collect_allowed_methods(&self, path: &str) -> SmallVec<[Method; 4]> {
    let mut allowed = SmallVec::<[Method; 4]>::new();
    for (method, m) in self.inner.iter() {
      if m.at(path).is_ok() {
        allowed.push(method);
      }
    }
    allowed
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

    for (method, weak_vec) in self.routes.iter() {
      for weak in weak_vec {
        if let Some(route) = weak.upgrade() {
          if let Some(openapi) = route.openapi_metadata() {
            result.push((method.clone(), route.path.clone(), openapi));
          }
        }
      }
    }

    result
  }
}

/// Joins a path prefix and a child path, normalising the boundary slash.
fn combine_prefix_path(prefix: &str, path: &str) -> String {
  if prefix.is_empty() || prefix == "/" {
    return path.to_string();
  }
  let prefix = prefix.trim_end_matches('/');
  if path.is_empty() || path == "/" {
    return prefix.to_string();
  }
  if path.starts_with('/') {
    let mut out = String::with_capacity(prefix.len() + path.len());
    out.push_str(prefix);
    out.push_str(path);
    out
  } else {
    let mut out = String::with_capacity(prefix.len() + 1 + path.len());
    out.push_str(prefix);
    out.push('/');
    out.push_str(path);
    out
  }
}

/// Joins a slice of HTTP methods into a comma-separated `Allow`-header value.
fn join_methods(methods: &[Method]) -> String {
  let mut out = String::with_capacity(methods.len() * 8);
  for (i, m) in methods.iter().enumerate() {
    if i > 0 {
      out.push_str(", ");
    }
    out.push_str(m.as_str());
  }
  out
}

/// Distributed slice of route registration thunks.
///
/// Each `#[tako::route]` / `#[tako::get]` / etc. attribute contributes a
/// `fn(&mut Router)` closure that calls [`Router::route`] with the
/// generated `Params::METHOD` / `Params::PATH` and the handler. Iterating
/// the slice — what [`Router::mount_all`] does — replays every contribution
/// against the supplied router.
#[linkme::distributed_slice]
pub static TAKO_ROUTES: [fn(&mut Router)] = [..];

/// Maps the 9 standard HTTP methods to array indices.
/// Returns `None` for non-standard / extension methods.
#[inline]
fn method_slot(method: &Method) -> Option<usize> {
  Some(match *method {
    Method::GET => 0,
    Method::POST => 1,
    Method::PUT => 2,
    Method::DELETE => 3,
    Method::PATCH => 4,
    Method::HEAD => 5,
    Method::OPTIONS => 6,
    Method::CONNECT => 7,
    Method::TRACE => 8,
    _ => return None,
  })
}

/// Reconstructs a `Method` from its slot index.
#[inline]
fn method_from_slot(idx: usize) -> Method {
  match idx {
    0 => Method::GET,
    1 => Method::POST,
    2 => Method::PUT,
    3 => Method::DELETE,
    4 => Method::PATCH,
    5 => Method::HEAD,
    6 => Method::OPTIONS,
    7 => Method::CONNECT,
    8 => Method::TRACE,
    _ => unreachable!(),
  }
}

/// A compact, cache-friendly map keyed by HTTP method.
///
/// Standard methods (GET, POST, PUT, …) use O(1) array indexing.
/// Non-standard methods fall back to linear scan (extremely rare in practice).
struct MethodMap<V> {
  standard: [Option<V>; 9],
  custom: Vec<(Method, V)>,
}

impl<V> MethodMap<V> {
  fn new() -> Self {
    Self {
      standard: std::array::from_fn(|_| None),
      custom: Vec::new(),
    }
  }

  /// O(1) lookup for standard methods, linear scan for custom.
  #[inline]
  fn get(&self, method: &Method) -> Option<&V> {
    if let Some(idx) = method_slot(method) {
      self.standard[idx].as_ref()
    } else {
      self
        .custom
        .iter()
        .find(|(m, _)| m == method)
        .map(|(_, v)| v)
    }
  }

  /// Returns a mutable reference, inserting `V::default()` if absent.
  fn get_or_default_mut(&mut self, method: &Method) -> &mut V
  where
    V: Default,
  {
    if let Some(idx) = method_slot(method) {
      self.standard[idx].get_or_insert_with(V::default)
    } else {
      let pos = self.custom.iter().position(|(m, _)| m == method);
      match pos {
        Some(pos) => &mut self.custom[pos].1,
        None => {
          self.custom.push((method.clone(), V::default()));
          &mut self.custom.last_mut().unwrap().1
        }
      }
    }
  }

  /// Iterates over all `(Method, &V)` pairs (standard then custom).
  fn iter(&self) -> impl Iterator<Item = (Method, &V)> {
    self
      .standard
      .iter()
      .enumerate()
      .filter_map(|(idx, slot)| slot.as_ref().map(|v| (method_from_slot(idx), v)))
      .chain(self.custom.iter().map(|(m, v)| (m.clone(), v)))
  }
}
