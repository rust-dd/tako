//! Middleware system for request and response processing pipelines.
//!
//! This module provides the core middleware infrastructure for Tako, allowing you to
//! compose request processing pipelines. Middleware can modify requests, responses,
//! or perform side effects like logging, authentication, or rate limiting. The `Next`
//! struct manages the execution flow through the middleware chain to the final handler.
//!
//! # Examples
//!
//! ```rust
//! use tako::{middleware::Next, types::{Request, Response}};
//! use std::{pin::Pin, future::Future};
//!
//! async fn middleware(req: Request, next: Next) -> Response {
//!     // Your logic here
//!     next.run(req).await
//! }
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::handler::BoxHandler;
use crate::types::BoxMiddleware;
use crate::types::Request;
use crate::types::Response;

pub mod api_key_auth;
pub mod basic_auth;
pub mod bearer_auth;
pub mod body_limit;
pub mod csrf;
pub mod jwt_auth;
pub mod request_id;
pub mod security_headers;
pub mod session;
pub mod upload_progress;

/// Trait for converting types into middleware functions.
///
/// This trait allows various types to be converted into middleware that can be used
/// in the Tako middleware pipeline. Middleware functions take a request and the next
/// middleware in the chain, returning a future that resolves to a response.
///
/// # Examples
///
/// ```rust
/// use tako::middleware::{IntoMiddleware, Next};
/// use tako::types::{Request, Response};
/// use std::{pin::Pin, future::Future};
///
/// struct LoggingMiddleware;
///
/// impl IntoMiddleware for LoggingMiddleware {
///     fn into_middleware(
///         self,
///     ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
///     + Clone + Send + Sync + 'static {
///         |req, next| {
///             Box::pin(async move {
///                 println!("Request: {}", req.uri());
///                 next.run(req).await
///             })
///         }
///     }
/// }
/// ```
#[doc(alias = "middleware")]
pub trait IntoMiddleware {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static;
}

/// Represents the next step in the middleware execution chain.
///
/// `Next` is passed to middleware functions to allow them to continue
/// the request processing chain. Calling `next.run(req)` will execute
/// the remaining middleware and eventually the endpoint handler.
#[doc(alias = "next")]
pub struct Next {
  /// Global middlewares to be executed before route-specific ones.
  pub global_middlewares: Arc<[BoxMiddleware]>,
  /// Route-specific middlewares executed after global ones.
  pub route_middlewares: Arc<[BoxMiddleware]>,
  /// Current position within the middleware chain.
  pub index: usize,
  /// Final endpoint handler to be called after all middlewares.
  pub endpoint: BoxHandler,
}

impl std::fmt::Debug for Next {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("Next")
      .field(
        "middlewares_remaining",
        &(self.global_middlewares.len() + self.route_middlewares.len()).saturating_sub(self.index),
      )
      .finish_non_exhaustive()
  }
}

impl Clone for Next {
  fn clone(&self) -> Self {
    Self {
      global_middlewares: Arc::clone(&self.global_middlewares),
      route_middlewares: Arc::clone(&self.route_middlewares),
      index: self.index,
      endpoint: self.endpoint.clone(),
    }
  }
}

impl Next {
  /// Executes the next middleware or endpoint in the chain.
  pub async fn run(mut self, req: Request) -> Response {
    let mw = if let Some(mw) = self.global_middlewares.get(self.index) {
      Some(mw.clone())
    } else {
      self
        .route_middlewares
        .get(self.index.saturating_sub(self.global_middlewares.len()))
        .cloned()
    };

    if let Some(mw) = mw {
      self.index += 1;
      mw(req, self).await
    } else {
      self.endpoint.call(req).await
    }
  }
}
