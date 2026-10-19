//! CORS: answers preflight requests and adds CORS headers for allowed origins, on the whole
//! router or a single route.
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
