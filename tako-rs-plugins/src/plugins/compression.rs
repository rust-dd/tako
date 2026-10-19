//! Response compression negotiated from `Accept-Encoding` (Brotli, Gzip, DEFLATE and, with the
//! `zstd` feature, Zstandard), skipped by content type, size and status.
//!
//! # Examples
//!
//! ```rust
//! use tako::plugins::compression::CompressionBuilder;
//! use tako::plugins::TakoPlugin;
//! use tako::router::Router;
//! use tako::Method;
//!
//! async fn handler(_req: tako::types::Request) -> &'static str {
//!     "Response data"
//! }
//!
//! async fn api_handler(_req: tako::types::Request) -> &'static str {
//!     "Large API response"
//! }
//!
//! let mut router = Router::new();
//!
//! // Router-level: Basic compression setup (applied to all routes)
//! let compression = CompressionBuilder::new()
//!     .enable_gzip(true)
//!     .enable_brotli(true)
//!     .min_size(1024)
//!     .build();
//! router.plugin(compression);
//!
//! // Route-level: Advanced compression for specific API endpoint
//! let api_route = router.route(Method::GET, "/api/large-data", api_handler);
//! let advanced = CompressionBuilder::new()
//!     .enable_gzip(true)
//!     .gzip_level(9)
//!     .enable_brotli(true)
//!     .brotli_level(11)
//!     .enable_stream(true)
//!     .min_size(512)
//!     .build();
//! api_route.plugin(advanced);
//! ```

pub mod brotli_stream;
mod builder;
mod config;
pub mod deflate_stream;
mod encoder;
mod encoding;
pub mod gzip_stream;
mod negotiate;
mod plugin;
#[cfg(test)]
mod tests;
pub mod zstd_stream;

pub use builder::CompressionBuilder;
pub use config::Config;
pub use config::ContentTypePolicy;
pub use encoding::Encoding;
pub use plugin::CompressionPlugin;
