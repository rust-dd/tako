//! The [`TakoPlugin`] trait, for features set up once on a router or on a single route.
//!
//! # Examples
//!
//! ```rust
//! use tako::plugins::TakoPlugin;
//! use tako::router::Router;
//! use tako::Method;
//! use anyhow::Result;
//!
//! #[derive(Clone)]
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

/// A reusable feature, such as CORS, that registers its middleware in `setup`.
///
/// # Examples
///
/// ```rust
/// use tako::plugins::TakoPlugin;
/// use tako::router::Router;
/// use tako::Method;
/// use anyhow::Result;
///
/// #[derive(Clone)]
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
  /// Identifies the plugin in setup error messages.
  fn name(&self) -> &'static str;

  /// Runs once; a route plugin gets a scratch router whose middleware moves to the route.
  fn setup(&self, router: &Router) -> Result<()>;
}
