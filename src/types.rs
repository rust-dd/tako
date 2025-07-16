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
///
/// This type alias represents an HTTP request with an `Incoming` body stream from Hyper,
/// providing efficient handling of request data including support for large payloads
/// and streaming content.
///
/// # Examples
///
/// ```rust
/// use tako::types::Request;
/// use tako::body::TakoBody;
///
/// async fn handle_request(req: Request) -> &'static str {
///     match req.method().as_str() {
///         "GET" => "Hello, World!",
///         "POST" => "Data received",
///         _ => "Method not allowed",
///     }
/// }
/// ```
pub type Request = hyper::Request<Incoming>;

/// HTTP response type with Tako's custom body implementation.
///
/// This type alias represents an HTTP response using `TakoBody` for efficient body
/// handling with support for various content types, streaming, and response composition.
///
/// # Examples
///
/// ```rust
/// use tako::types::Response;
/// use tako::body::TakoBody;
/// use http::StatusCode;
///
/// fn create_response() -> Response {
///     let mut response = Response::new(TakoBody::from("Success"));
///     *response.status_mut() = StatusCode::OK;
///     response.headers_mut().insert("content-type", "text/plain".parse().unwrap());
///     response
/// }
/// ```
pub type Response = hyper::Response<TakoBody>;

/// Boxed HTTP body type for internal response handling.
///
/// Internal type alias combining byte streams with error handling for HTTP response
/// bodies. Used internally by the framework for efficient body composition and
/// error propagation.
pub(crate) type BoxBody = UnsyncBoxBody<Bytes, BoxError>;

/// Boxed error type for thread-safe error handling.
///
/// Internal type alias for errors that can be sent across threads and support
/// dynamic dispatch. Used throughout the framework for consistent error handling
/// patterns.
pub(crate) type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Boxed middleware function type for dynamic middleware composition.
///
/// This type alias represents a middleware function wrapped in an `Arc` for shared
/// ownership and thread safety. Middleware functions take a request and the next
/// middleware in the chain, returning a future that resolves to a response.
///
/// # Examples
///
/// ```rust
/// use tako::types::{Request, Response, BoxMiddleware};
/// use tako::middleware::Next;
/// use std::sync::Arc;
///
/// // Create a logging middleware
/// let logging_middleware: BoxMiddleware = Arc::new(|req, next| {
///     Box::pin(async move {
///         println!("Processing request: {} {}", req.method(), req.uri());
///         let response = next.run(req).await;
///         println!("Response status: {}", response.status());
///         response
///     })
/// });
///
/// // Create an authentication middleware
/// let auth_middleware: BoxMiddleware = Arc::new(|req, next| {
///     Box::pin(async move {
///         if req.headers().contains_key("authorization") {
///             next.run(req).await
///         } else {
///             Response::builder()
///                 .status(401)
///                 .body(tako::body::TakoBody::from("Unauthorized"))
///                 .unwrap()
///         }
///     })
/// });
/// ```
pub type BoxMiddleware = Arc<dyn Fn(Request, Next) -> BoxFuture<'static, Response> + Send + Sync>;
