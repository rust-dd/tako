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
    sync::{Arc, RwLock, Weak},
};

use dashmap::DashMap;
use http::StatusCode;
use hyper::Method;

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
pub struct Router {
    /// Map of registered routes keyed by method.
    inner: DashMap<Method, matchit::Router<Arc<Route>>>,
    /// An easy-to-iterate index of the same routes so we can access the `Arc<Route>` values
    routes: DashMap<Method, Vec<Weak<Route>>>,
    /// Global middleware chain applied to all routes.
    middlewares: RwLock<Vec<BoxMiddleware>>,
    /// Registered plugins for extending functionality.
    #[cfg(feature = "plugins")]
    plugins: Vec<Box<dyn TakoPlugin>>,
    /// Flag to ensure plugins are initialized only once.
    #[cfg(feature = "plugins")]
    plugins_initialized: AtomicBool,
}

impl Router {
    /// Creates a new, empty router.
    pub fn new() -> Self {
        Self {
            inner: DashMap::default(),
            routes: DashMap::default(),
            middlewares: RwLock::new(Vec::new()),
            #[cfg(feature = "plugins")]
            plugins: Vec::new(),
            #[cfg(feature = "plugins")]
            plugins_initialized: AtomicBool::new(false),
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
    pub fn route<H>(&mut self, method: Method, path: &str, handler: H) -> Arc<Route>
    where
        H: Handler + Clone + 'static,
    {
        let route = Arc::new(Route::new(
            path.to_string(),
            method.clone(),
            BoxHandler::new(handler),
            None,
        ));

        let mut method_router = self
            .inner
            .entry(method.clone())
            .or_insert_with(matchit::Router::new);

        if let Err(err) = method_router.insert(path.to_string(), route.clone()) {
            panic!("Failed to register route: {}", err);
        }

        self.routes
            .entry(method)
            .or_insert_with(Vec::new)
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
    pub fn route_with_tsr<H>(&mut self, method: Method, path: &str, handler: H) -> Arc<Route>
    where
        H: Handler + Clone + 'static,
    {
        if path == "/" {
            panic!("Cannot route with TSR for root path");
        }

        let route = Arc::new(Route::new(
            path.to_string(),
            method.clone(),
            BoxHandler::new(handler),
            Some(true),
        ));

        let mut method_router = self
            .inner
            .entry(method.clone())
            .or_insert_with(matchit::Router::new);

        if let Err(err) = method_router.insert(path.to_string(), route.clone()) {
            panic!("Failed to register route: {}", err);
        }

        self.routes
            .entry(method)
            .or_insert_with(Vec::new)
            .push(Arc::downgrade(&route));

        route
    }

    /// Dispatches an incoming request to the appropriate route handler.
    pub async fn dispatch(&self, mut req: Request) -> Response {
        let method = req.method();
        let path = req.uri().path();

        if let Some(method_router) = self.inner.get(method) {
            if let Ok(matched) = method_router.at(path) {
                let route = matched.value;

                if !matched.params.iter().collect::<Vec<_>>().is_empty() {
                    let mut params = HashMap::new();
                    for (k, v) in matched.params.iter() {
                        params.insert(k.to_string(), v.to_string());
                    }
                    req.extensions_mut().insert(PathParams(params));
                }
                let g_mws = self.middlewares.read().unwrap().clone();
                let r_mws = route.middlewares.read().unwrap().clone();
                let mut chain = Vec::new();
                chain.extend(g_mws.into_iter());
                chain.extend(r_mws.into_iter());

                let next = Next {
                    middlewares: Arc::new(chain),
                    endpoint: Arc::new(route.handler.clone()),
                };
                return next.run(req).await;
            }
        }

        let tsr_path = if path.ends_with('/') {
            path.trim_end_matches('/').to_string()
        } else {
            format!("{}/", path)
        };

        if let Some(method_router) = self.inner.get(method) {
            if let Ok(matched) = method_router.at(&tsr_path) {
                if matched.value.tsr {
                    return hyper::Response::builder()
                        .status(StatusCode::TEMPORARY_REDIRECT)
                        .header("Location", tsr_path)
                        .body(TakoBody::empty())
                        .unwrap();
                }
            }
        }

        hyper::Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(TakoBody::empty())
            .unwrap()
    }

    /// Adds a value to the global state accessible by all handlers.
    ///
    /// Global state allows sharing data across different routes and middleware.
    /// Values are stored by string keys and can be retrieved in handlers using
    /// the state extraction functionality.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::router::Router;
    ///
    /// #[derive(Clone)]
    /// struct AppConfig {
    ///     database_url: String,
    ///     api_key: String,
    /// }
    ///
    /// let mut router = Router::new();
    /// router.state("config", AppConfig {
    ///     database_url: "postgresql://localhost/mydb".to_string(),
    ///     api_key: "secret-key".to_string(),
    /// });
    /// router.state("version", "1.0.0".to_string());
    /// ```
    pub fn state<T: Clone + Send + Sync + 'static>(&mut self, key: &str, value: T) {
        set_state(key, value);
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

        self.middlewares.write().unwrap().push(mw);
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
    pub fn plugin<P>(&mut self, plugin: P) -> &mut Self
    where
        P: TakoPlugin + Clone + Send + Sync + 'static,
    {
        self.plugins.push(Box::new(plugin));
        self
    }

    /// Returns references to all registered plugins.
    #[cfg(feature = "plugins")]
    pub(crate) fn plugins(&self) -> Vec<&dyn TakoPlugin> {
        self.plugins.iter().map(|plugin| plugin.as_ref()).collect()
    }

    /// Initializes all registered plugins exactly once.
    #[cfg(feature = "plugins")]
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
        let upstream_globals = other.middlewares.read().unwrap().clone();

        for (method, weak_vec) in other.routes.into_iter() {
            let mut target_router = self
                .inner
                .entry(method.clone())
                .or_insert_with(matchit::Router::new);

            for weak in weak_vec {
                if let Some(route) = weak.upgrade() {
                    let mut rmw = route.middlewares.write().unwrap();
                    for mw in upstream_globals.iter().rev() {
                        rmw.push_front(mw.clone());
                    }

                    let _ = target_router.insert(route.path.clone(), route.clone());

                    self.routes
                        .entry(method.clone())
                        .or_insert_with(Vec::new)
                        .push(Arc::downgrade(&route));
                }
            }
        }
    }
}
