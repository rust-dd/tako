//! [`Next`], the rest of a middleware chain, and the [`IntoMiddleware`] conversion.
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

/// Converts a configured value into a middleware closure for `Router::middleware`.
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

/// The rest of the chain; `next.run(req)` runs the remaining middleware, then the handler.
#[doc(alias = "next")]
pub struct Next {
  pub(crate) global_middlewares: Arc<Vec<BoxMiddleware>>,
  pub(crate) route_middlewares: Arc<Vec<BoxMiddleware>>,
  /// Position in `global_middlewares` followed by `route_middlewares`.
  pub(crate) index: usize,
  pub(crate) endpoint: BoxHandler,
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
