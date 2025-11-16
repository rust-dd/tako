#![cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
//! Plugin system for extending framework functionality with composable modules.
//!
//! This module provides the core plugin infrastructure for Tako, allowing developers
//! to extend the framework with reusable components. Plugins can add middleware,
//! modify routing behavior, or integrate external services. The `TakoPlugin` trait
//! defines the interface all plugins must implement for registration and setup.
//!
//! Plugins can be applied at two levels:
//! - **Router-level**: Applied globally to all routes using `router.plugin()`
//! - **Route-level**: Applied to specific routes using `route.plugin()`
//!
//! # Examples
//!
//! ```rust
//! use tako::plugins::TakoPlugin;
//! use tako::router::Router;
//! use tako::Method;
//! use anyhow::Result;
//!
//! struct LoggingPlugin {
//!     level: String,
//! }
//!
//! impl TakoPlugin for LoggingPlugin {
//!     fn name(&self) -> &'static str {
//!         "logging"
//!     }
//!
//!     fn setup(&self, _router: &Router) -> Result<()> {
//!         println!("Setting up logging plugin with level: {}", self.level);
//!         Ok(())
//!     }
//! }
//!
//! async fn handler(_req: tako::types::Request) -> &'static str {
//!     "Hello"
//! }
//!
//! // Router-level plugin (applied to all routes)
//! let mut router = Router::new();
//! router.plugin(LoggingPlugin { level: "info".to_string() });
//!
//! // Route-level plugin (applied to specific route only)
//! let route = router.route(Method::GET, "/api/data", handler);
//! route.plugin(LoggingPlugin { level: "debug".to_string() });
//! ```

use anyhow::Result;

use crate::router::Router;

/// Compression plugin for automatic response compression.
pub mod compression;

/// CORS (Cross-Origin Resource Sharing) plugin for handling cross-origin requests.
pub mod cors;

/// Rate limiting plugin for controlling request frequency.
pub mod rate_limiter;

/// Metrics/tracing plugin for integrating with systems like Prometheus or OpenTelemetry.
pub mod metrics;

/// Trait for implementing Tako framework plugins.
///
/// Plugins extend the framework's functionality by implementing this trait. They can
/// add middleware, modify routing behavior, register handlers, or integrate external
/// services. All plugins must be thread-safe and have a static lifetime.
///
/// Plugins can be applied at both router and route levels:
/// - **Router-level**: Use `router.plugin()` to apply globally
/// - **Route-level**: Use `route.plugin()` to apply to specific routes
///
/// # Examples
///
/// ```rust
/// use tako::plugins::TakoPlugin;
/// use tako::router::Router;
/// use tako::Method;
/// use anyhow::Result;
///
/// struct CachePlugin {
///     ttl_seconds: u64,
/// }
///
/// impl TakoPlugin for CachePlugin {
///     fn name(&self) -> &'static str {
///         "cache"
///     }
///
///     fn setup(&self, router: &Router) -> Result<()> {
///         // Add middleware to the router
///         router.middleware(|req, next| async move {
///             // Cache logic here
///             next.run(req).await
///         });
///         Ok(())
///     }
/// }
///
/// async fn handler(_req: tako::types::Request) -> &'static str {
///     "Hello"
/// }
///
/// // Router-level usage
/// let mut router = Router::new();
/// router.plugin(CachePlugin { ttl_seconds: 300 });
///
/// // Route-level usage
/// let route = router.route(Method::GET, "/api/data", handler);
/// route.plugin(CachePlugin { ttl_seconds: 600 });
/// ```
pub trait TakoPlugin: Send + Sync + 'static {
  /// Returns the unique name identifier for this plugin.
  fn name(&self) -> &'static str;

  /// Configures and initializes the plugin with the given router.
  fn setup(&self, router: &Router) -> Result<()>;
}
