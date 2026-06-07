#![cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
//! Cross-Origin Resource Sharing (CORS) plugin for handling cross-origin HTTP requests.
//!
//! This module provides comprehensive CORS support for Tako web applications, enabling
//! secure cross-origin resource sharing between different domains. The plugin handles
//! preflight OPTIONS requests, validates origins against configured policies, and adds
//! appropriate CORS headers to responses. It supports configurable origins, methods,
//! headers, credentials, and cache control for flexible cross-origin access policies.
//!
//! The CORS plugin can be applied at both router-level (all routes) and route-level
//! (specific routes), allowing fine-grained control over CORS policies.
//!
//! # Examples
//!
//! ```rust
//! use tako::plugins::cors::{CorsPlugin, CorsBuilder};
//! use tako::plugins::TakoPlugin;
//! use tako::router::Router;
//! use http::Method;
//!
//! async fn api_handler(_req: tako::types::Request) -> &'static str {
//!     "API response"
//! }
//!
//! async fn public_handler(_req: tako::types::Request) -> &'static str {
//!     "Public response"
//! }
//!
//! let mut router = Router::new();
//!
//! // Router-level: Basic CORS setup allowing all origins (applied to all routes)
//! let global_cors = CorsBuilder::new().build();
//! router.plugin(global_cors);
//!
//! // Route-level: Restrictive CORS for specific API endpoint
//! let api_route = router.route(Method::GET, "/api/data", api_handler);
//! let api_cors = CorsBuilder::new()
//!     .allow_origin("https://app.example.com")
//!     .allow_origin("https://admin.example.com")
//!     .allow_methods(&[Method::GET, Method::POST, Method::PUT])
//!     .allow_credentials(true)
//!     .max_age_secs(86400)
//!     .build();
//! api_route.plugin(api_cors);
//!
//! // Another route without CORS restrictions (uses global if set)
//! router.route(Method::GET, "/public", public_handler);
//! ```

mod builder;
mod config;
mod middleware;
mod origin;
mod plugin;

pub use builder::CorsBuilder;
pub use config::Config;
pub use config::CorsConfigError;
pub use origin::OriginMatcher;
pub use plugin::CorsPlugin;
