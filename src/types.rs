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
use hyper::body::Incoming;

use crate::{body::TakoBody, middleware::Next};

/// HTTP request type with streaming body support.
pub type Request = http::Request<Incoming>;

/// HTTP response type with Tako's custom body implementation.
pub type Response = http::Response<TakoBody>;

/// Boxed HTTP body type for internal response handling.
pub(crate) type BoxBody = UnsyncBoxBody<Bytes, BoxError>;

/// Boxed error type for thread-safe error handling.
pub(crate) type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Boxed middleware function type for dynamic middleware composition.
pub type BoxMiddleware = Arc<dyn Fn(Request, Next) -> BoxFuture<'static, Response> + Send + Sync>;
