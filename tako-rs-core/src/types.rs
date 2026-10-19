//! Request, response and middleware type aliases used across the framework.
//!
//! # Examples
//!
//! ```rust
//! use tako::types::{Request, Response, BoxMiddleware};
//! use tako::middleware::Next;
//! use std::sync::Arc;
//!
//! // Using the Request type in a handler
//! async fn handler(req: Request) -> Response {
//!     Response::new(tako::body::TakoBody::from("Hello, World!"))
//! }
//!
//! // Creating middleware using BoxMiddleware
//! let middleware: BoxMiddleware = Arc::new(|req, next| {
//!     Box::pin(async move {
//!         println!("Request to: {}", req.uri());
//!         next.run(req).await
//!     })
//! });
//! ```

use std::sync::Arc;

use bytes::Bytes;
use futures_util::future::BoxFuture;
use http_body_util::combinators::UnsyncBoxBody;

use crate::body::TakoBody;
use crate::middleware::Next;

/// HTTP request type with streaming body support (hyper-independent).
pub type Request = http::Request<TakoBody>;

/// HTTP response type with Tako's custom body implementation.
pub type Response = http::Response<TakoBody>;

/// The boxed form of `TakoBody`: `Send` but not `Sync`, so stream sources need not be `Sync`.
#[doc(hidden)]
pub type BoxBody = UnsyncBoxBody<Bytes, BoxError>;

/// The error type of `TakoBody` frames.
#[doc(hidden)]
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Boxed middleware function type for dynamic middleware composition.
pub type BoxMiddleware = Arc<dyn Fn(Request, Next) -> BoxFuture<'static, Response> + Send + Sync>;

#[cfg(feature = "ahash")]
pub type BuildHasher = ahash::RandomState;

#[cfg(not(feature = "ahash"))]
pub type BuildHasher = std::collections::hash_map::RandomState;
