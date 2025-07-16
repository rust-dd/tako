//! Plugin system for extending framework functionality with composable modules.
//!
//! This module provides the core plugin infrastructure for Tako, allowing developers
//! to extend the framework with reusable components. Plugins can add middleware,
//! modify routing behavior, or integrate external services. The `TakoPlugin` trait
//! defines the interface all plugins must implement for registration and setup.
//!
//! # Examples
//!
//! ```rust
//! use tako::plugins::TakoPlugin;
//! use tako::router::Router;
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
//! let plugin = LoggingPlugin { level: "info".to_string() };
//! assert_eq!(plugin.name(), "logging");
//! ```

use anyhow::Result;

use crate::router::Router;

/// Compression plugin for automatic response compression.
pub mod compression;

/// CORS (Cross-Origin Resource Sharing) plugin for handling cross-origin requests.
pub mod cors;

/// Rate limiting plugin for controlling request frequency.
pub mod rate_limiter;

/// Trait for implementing Tako framework plugins.
///
/// Plugins extend the framework's functionality by implementing this trait. They can
/// add middleware, modify routing behavior, register handlers, or integrate external
/// services. All plugins must be thread-safe and have a static lifetime.
///
/// # Examples
///
/// ```rust
/// use tako::plugins::TakoPlugin;
/// use tako::router::Router;
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
///     fn setup(&self, _router: &Router) -> Result<()> {
///         println!("Configuring cache with TTL: {} seconds", self.ttl_seconds);
///         Ok(())
///     }
/// }
///
/// let plugin = CachePlugin { ttl_seconds: 300 };
/// let router = Router::new();
/// plugin.setup(&router).unwrap();
/// ```
pub trait TakoPlugin: Send + Sync + 'static {
    /// Returns the unique name identifier for this plugin.
    fn name(&self) -> &'static str;

    /// Configures and initializes the plugin with the given router.
    fn setup(&self, router: &Router) -> Result<()>;
}
