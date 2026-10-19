//! The [`Route`] that router registration returns, for configuring that one route.
//!
//! # Examples
//!
//! ```rust,no_run
//! use tako::{router::Router, types::Request};
//! use http::Method;
//!
//! async fn user_handler(_req: Request) -> &'static str {
//!   "user profile"
//! }
//!
//! // `Route` is constructed indirectly through the router. The route's
//! // middleware / plugin / timeout / signal configuration is then chained
//! // off the returned `&Route` reference. Path matching happens inside the
//! // router and is not exposed on `Route` directly.
//! let mut router = Router::new();
//! router
//!   .route(Method::GET, "/users/{id}", user_handler)
//!   .timeout(std::time::Duration::from_secs(5));
//! ```

mod builder;
mod def;
#[cfg(any(feature = "utoipa", feature = "vespera"))]
mod openapi;

pub use def::Route;
