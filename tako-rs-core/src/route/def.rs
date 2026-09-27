//! The [`Route`] struct, its handler/middleware storage, and construction.
//!
//! Holds the field layout for a route (path, method, handler, middleware
//! chain, protocol guard, and feature-gated plugin / signal / `OpenAPI`
//! state) plus the constructor and the `cloned_with_path` helper used by
//! the router to re-home routes under a prefix.

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
#[cfg(any(feature = "utoipa", feature = "vespera"))]
use crate::openapi::RouteOpenApi;
#[cfg(feature = "plugins")]
use crate::plugins::TakoPlugin;
#[cfg(feature = "signals")]
use crate::signals::SignalArbiter;
use crate::types::BoxMiddleware;

/// HTTP route with path pattern matching and middleware support.
#[doc(alias = "route")]
pub struct Route {
  /// Original path string used to create this route.
  pub path: Arc<str>,
  pub(crate) parameter_keys: Vec<Arc<str>>,
  /// HTTP method this route responds to.
  pub method: Method,
  /// Handler function to execute when route is matched.
  ///
  pub(crate) handler: BoxHandler,
  /// Route-specific middleware chain.
  ///
  /// Crate-private to protect the `has_middleware` shortcut: every mutation
  /// must go through [`Route::middleware`] so the atomic flag stays in sync
  /// with the `ArcSwap` contents. Direct `ArcSwap::store` from outside would
  /// silently desynchronize the hot-path skip in the router.
  pub(crate) middlewares: ArcSwap<Vec<BoxMiddleware>>,
  /// Fast check: true when route middleware is registered (avoids `ArcSwap` load on hot path).
  pub(crate) has_middleware: AtomicBool,
  /// Whether trailing slash redirection is enabled.
  pub tsr: bool,
  /// Route-specific plugins.
  #[cfg(feature = "plugins")]
  pub(crate) plugins: RwLock<Vec<Arc<dyn TakoPlugin>>>,
  /// Publishes plugin middleware before concurrent requests can proceed.
  #[cfg(feature = "plugins")]
  pub(crate) plugins_initialized: OnceLock<Result<(), String>>,
  /// HTTP protocol version guard (set once via [`Route::version`] / `h09`/`h10`/`h11`/`h2`).
  pub(crate) http_protocol: OnceLock<http::Version>,
  /// Route-level signal arbiter.
  #[cfg(feature = "signals")]
  pub(crate) signals: SignalArbiter,
  /// `OpenAPI` metadata for this route.
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  pub(crate) openapi: RwLock<Option<RouteOpenApi>>,
  /// Route-specific timeout override (set once at registration, lock-free reads).
  pub(crate) timeout: OnceLock<Duration>,
  /// Route-level SIMD JSON dispatch mode (set once at registration, lock-free reads).
  pub(crate) simd_json_mode: OnceLock<SimdJsonMode>,
  pub(crate) scoped_state: Option<Arc<crate::router_state::RouterState>>,
  pub(crate) scoped_timeout: Option<Arc<crate::router::Router>>,
  pub(crate) body_limit: Option<crate::extractors::body::BodyLimit>,
}

impl Route {
  /// Creates a new route with the specified path, method, and handler.
  pub fn new(path: String, method: Method, handler: BoxHandler, tsr: Option<bool>) -> Self {
    Self {
      parameter_keys: parameter_keys(&path),
      path: path.into(),
      method,
      handler,
      middlewares: ArcSwap::new(Arc::default()),
      has_middleware: AtomicBool::new(false),
      tsr: tsr.unwrap_or(false),
      #[cfg(feature = "plugins")]
      plugins: RwLock::new(Vec::new()),
      #[cfg(feature = "plugins")]
      plugins_initialized: OnceLock::new(),
      http_protocol: OnceLock::new(),
      #[cfg(feature = "signals")]
      signals: SignalArbiter::new(),
      #[cfg(any(feature = "utoipa", feature = "vespera"))]
      openapi: RwLock::new(None),
      timeout: OnceLock::new(),
      simd_json_mode: OnceLock::new(),
      scoped_state: None,
      scoped_timeout: None,
      body_limit: None,
    }
  }

  /// Builds a new `Arc<Route>` with the same handler / middlewares / config
  /// but a different path. Used by [`crate::router::Router::nest`] to register
  /// a child router's routes under a prefix without mutating the originals.
  ///
  /// Plugin instances are shared; an initialized middleware chain is not installed twice.
  pub(crate) fn cloned_with_path(&self, new_path: String) -> Arc<Route> {
    let cloned = Self {
      parameter_keys: parameter_keys(&new_path),
      path: new_path.into(),
      scoped_state: self.scoped_state.clone(),
      scoped_timeout: self.scoped_timeout.clone(),
      body_limit: self.body_limit,
      method: self.method.clone(),
      handler: self.handler.clone(),
      middlewares: ArcSwap::new(self.middlewares.load_full()),
      has_middleware: AtomicBool::new(self.has_middleware.load(Ordering::Acquire)),
      tsr: self.tsr,
      #[cfg(feature = "plugins")]
      plugins: RwLock::new(self.plugins.read().clone()),
      #[cfg(feature = "plugins")]
      plugins_initialized: self.plugins_initialized.clone(),
      http_protocol: {
        let lock = OnceLock::new();
        if let Some(v) = self.http_protocol.get() {
          let _ = lock.set(*v);
        }
        lock
      },
      // Preserve the source route's signal handlers across `cloned_with_path`
      // (nest/mount): allocating a fresh `SignalArbiter` here used to silently
      // drop every handler the caller had registered on the original route.
      #[cfg(feature = "signals")]
      signals: self.signals.clone(),
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

fn parameter_keys(path: &str) -> Vec<Arc<str>> {
  path
    .split('{')
    .skip(1)
    .filter_map(|segment| {
      segment
        .split_once('}')
        .map(|(key, _)| Arc::from(key.trim_start_matches('*')))
    })
    .collect()
}
