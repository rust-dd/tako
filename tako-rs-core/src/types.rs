//! Core type definitions and aliases used throughout the Tako framework.
//!
//! This module provides fundamental type aliases that standardize the types used across
//! the framework for requests, responses, errors, and middleware. These aliases ensure
//! consistency and make the API more ergonomic by hiding complex generic parameters.
//! The main types include `Request` and `Response` for HTTP handling, and `BoxMiddleware`
//! for middleware function composition.
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

/// Boxed HTTP body alias used inside `TakoBody::Boxed`.
///
/// Backed by `http_body_util::UnsyncBoxBody`, which is `Send` but **not**
/// `Sync` — this matches how hyper passes bodies between tasks (each task
/// owns the body exclusively) and avoids requiring `Sync` from streaming
/// sources, which is uncommon for `tokio::sync::mpsc::Receiver`-driven
/// pipelines. Code that needs to move a body across threads should clone
/// the higher-level `TakoBody`, not the raw `BoxBody`.
#[doc(hidden)]
pub type BoxBody = UnsyncBoxBody<Bytes, BoxError>;

/// Error type used inside boxed body / middleware error channels.
///
/// `Send + Sync` because errors travel across `await` points and may be
/// inspected by a different task than the one that produced them. This is a
/// container — values are usually framework-internal `std::io::Error` /
/// `hyper::Error` instances wrapped on the way back to a `Responder`. Not a
/// thread-safety primitive on its own; do not rely on the `Sync` bound for
/// shared-mutable error tracking.
#[doc(hidden)]
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Boxed middleware function type for dynamic middleware composition.
pub type BoxMiddleware = Arc<dyn Fn(Request, Next) -> BoxFuture<'static, Response> + Send + Sync>;

#[cfg(feature = "ahash")]
pub type BuildHasher = ahash::RandomState;

#[cfg(not(feature = "ahash"))]
pub type BuildHasher = std::collections::hash_map::RandomState;
