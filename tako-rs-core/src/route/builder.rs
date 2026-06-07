//! The chainable route configuration API.
//!
//! These `&self -> &Self` builder methods are chained off the `&Route`
//! returned by the router: middleware and plugin registration, the HTTP
//! protocol-version guard, per-route signal wiring, the timeout override,
//! and the SIMD JSON dispatch mode — plus the crate-private getters the
//! dispatch hot path reads back.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::Route;
use crate::extractors::json::SimdJsonMode;
use crate::middleware::Next;
#[cfg(feature = "plugins")]
use crate::plugins::TakoPlugin;
use crate::responder::Responder;
#[cfg(feature = "signals")]
use crate::signals::Signal;
#[cfg(feature = "signals")]
use crate::signals::SignalArbiter;
use crate::types::BoxMiddleware;
use crate::types::Request;

impl Route {
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
    // Hot path: dispatch calls this on every matched route. After the
    // first request the plugins are installed, so do a cheap Acquire
    // load and bail before paying for a SeqCst RMW + fence each time.
    // `Acquire` pairs with the `Release` half of the `swap` below so we
    // observe the middleware writes published by the initializing thread.
    if self.plugins_initialized.load(Ordering::Acquire) {
      return;
    }

    if !self.plugins_initialized.swap(true, Ordering::SeqCst) {
      // Create a temporary mini-router to capture plugin middleware
      let mini_router = crate::router::Router::new();

      let plugins = self.plugins.read();
      for plugin in plugins.iter() {
        // See `Router::setup_plugins_once`: log failures so an erroring
        // route-level plugin (auth, rate-limit, ...) is visible instead
        // of silently dropped — fail-open without diagnostics is
        // exactly what the audit calls out.
        if let Err(e) = plugin.setup(&mini_router) {
          tracing::error!(
            plugin = plugin.name(),
            error = %e,
            "route-level TakoPlugin::setup failed; plugin not active"
          );
        }
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
    if let Err(_existing) = self.http_protocol.set(version) {
      tracing::warn!(
        path = %self.path,
        method = ?self.method,
        existing = ?self.http_protocol.get().copied(),
        requested = ?version,
        "Route::version called twice; subsequent calls are ignored (OnceLock first-wins)",
      );
    }
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
    if let Err(_existing) = self.timeout.set(duration) {
      tracing::warn!(
        path = %self.path,
        method = ?self.method,
        existing_ms = self.timeout.get().copied().unwrap_or_default().as_millis() as u64,
        requested_ms = duration.as_millis() as u64,
        "Route::timeout called twice; subsequent calls are ignored (OnceLock first-wins)",
      );
    }
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
    if let Err(_existing) = self.simd_json_mode.set(mode) {
      tracing::warn!(
        path = %self.path,
        method = ?self.method,
        existing = ?self.simd_json_mode.get().copied(),
        requested = ?mode,
        "Route::simd_json called twice; subsequent calls are ignored (OnceLock first-wins)",
      );
    }
    self
  }

  /// Returns the configured SIMD JSON mode for this route, if any.
  #[inline]
  pub(crate) fn get_simd_json_mode(&self) -> Option<SimdJsonMode> {
    self.simd_json_mode.get().copied()
  }
}
