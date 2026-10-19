//! Request body limits: oversized `Content-Length` is rejected with 413 up front, and the
//! body is capped while it streams.
//!
//! # Examples
//!
//! ```rust
//! use tako::middleware::body_limit::BodyLimit;
//! use tako::middleware::IntoMiddleware;
//!
//! // Static 1MB limit for all requests
//! let limit = BodyLimit::new(1024 * 1024);
//! let middleware = limit.into_middleware();
//!
//! // Dynamic limit based on request properties
//! let dynamic = BodyLimit::with_dynamic_limit(|req| {
//!     if req.uri().path().starts_with("/upload") {
//!         50 * 1024 * 1024 // 50MB for uploads
//!     } else {
//!         1024 * 1024 // 1MB for other requests
//!     }
//! });
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::StatusCode;
use http::header::CONTENT_LENGTH;
use http_body_util::Limited;
use tako_rs_core::body::TakoBody;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

/// Limits are in bytes.
///
/// # Examples
///
/// ```rust
/// use tako::middleware::body_limit::BodyLimit;
/// use tako::types::Request;
///
/// // Simple static limit
/// let static_limit = BodyLimit::new(1024 * 1024); // 1MB
///
/// // Dynamic limit based on endpoint
/// let dynamic_limit = BodyLimit::with_dynamic_limit(|req| {
///     match req.uri().path() {
///         "/api/upload" => 10 * 1024 * 1024, // 10MB for uploads
///         "/api/data" => 5 * 1024 * 1024,    // 5MB for data
///         _ => 1024 * 1024,                  // 1MB default
///     }
/// });
/// ```
pub struct BodyLimit<F = fn(&Request) -> usize>
where
  F: Fn(&Request) -> usize + Send + Sync + 'static,
{
  limit: Limit<F>,
}

#[derive(Clone)]
enum Limit<F> {
  Static(usize),
  Dynamic(F),
}

impl BodyLimit<fn(&Request) -> usize> {
  pub fn new(limit: usize) -> Self {
    Self {
      limit: Limit::Static(limit),
    }
  }
}

impl<F> BodyLimit<F>
where
  F: Fn(&Request) -> usize + Send + Sync + 'static,
{
  pub fn with_dynamic_limit(f: F) -> Self {
    Self {
      limit: Limit::Dynamic(f),
    }
  }
}

impl<F> IntoMiddleware for BodyLimit<F>
where
  F: Fn(&Request) -> usize + Send + Sync + 'static,
{
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let limit = match self.limit {
      Limit::Static(n) => Limit::Static(n),
      Limit::Dynamic(f) => Limit::Dynamic(Arc::new(f)),
    };

    move |req: Request, next: Next| {
      let limit = limit.clone();

      Box::pin(async move {
        let limit = match &limit {
          Limit::Static(n) => *n,
          Limit::Dynamic(f) => f(&req),
        };

        if let Some(len) = req
          .headers()
          .get(CONTENT_LENGTH)
          .and_then(|v| v.to_str().ok())
          .and_then(|s| s.parse::<usize>().ok())
          && len > limit
        {
          return (StatusCode::PAYLOAD_TOO_LARGE, "Body exceeds allowed size").into_response();
        }

        // Reads past `limit` fail with `LengthLimitError`; handlers can downcast it to answer 413.
        let (parts, body) = req.into_parts();
        let limited = Limited::new(body, limit);
        let req = http::Request::from_parts(parts, TakoBody::new(limited));
        next.run(req).await.into_response()
      })
    }
  }
}
