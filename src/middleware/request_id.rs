//! Request ID middleware for tracing and correlation.
//!
//! Generates or propagates a unique request identifier via the `X-Request-ID` header.
//! If the incoming request already has the header, it is preserved; otherwise a new
//! UUID v4 is generated. The ID is injected into both request extensions and
//! the response header.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::HeaderName;
use http::HeaderValue;

use crate::middleware::IntoMiddleware;
use crate::middleware::Next;
use crate::types::Request;
use crate::types::Response;

/// A request ID value that can be extracted from request extensions.
#[derive(Debug, Clone)]
pub struct RequestIdValue(pub String);

/// Request ID middleware configuration.
///
/// # Examples
///
/// ```rust
/// use tako::middleware::request_id::RequestId;
/// use tako::middleware::IntoMiddleware;
///
/// // Default: uses X-Request-ID header with UUID v4
/// let mw = RequestId::new().into_middleware();
///
/// // Custom header name
/// let mw = RequestId::new().header_name("X-Correlation-ID").into_middleware();
/// ```
pub struct RequestId {
  header: HeaderName,
  generator: Arc<dyn Fn() -> String + Send + Sync + 'static>,
}

impl Default for RequestId {
  fn default() -> Self {
    Self::new()
  }
}

impl RequestId {
  /// Creates a new RequestId middleware with default settings (X-Request-ID, UUID v4).
  pub fn new() -> Self {
    Self {
      header: HeaderName::from_static("x-request-id"),
      generator: Arc::new(|| {
        // Simple UUID v4 generation without dependency
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
          .duration_since(UNIX_EPOCH)
          .unwrap_or_default();
        let seed = now.as_nanos();
        // Simple pseudo-random ID (good enough for request correlation)
        format!(
          "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
          (seed & 0xFFFFFFFF) as u32,
          ((seed >> 32) & 0xFFFF) as u16,
          ((seed >> 48) & 0x0FFF) as u16,
          (0x8000 | ((seed >> 60) & 0x3FFF)) as u16,
          (seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) & 0xFFFFFFFFFFFF) as u64,
        )
      }),
    }
  }

  /// Sets a custom header name for the request ID.
  pub fn header_name(mut self, name: &'static str) -> Self {
    self.header = HeaderName::from_static(name);
    self
  }

  /// Sets a custom ID generator function.
  pub fn generator(mut self, f: impl Fn() -> String + Send + Sync + 'static) -> Self {
    self.generator = Arc::new(f);
    self
  }
}

impl IntoMiddleware for RequestId {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let header = self.header;
    let generator = self.generator;

    move |mut req: Request, next: Next| {
      let header = header.clone();
      let generator = generator.clone();

      Box::pin(async move {
        // Use existing request ID or generate a new one
        let id = req
          .headers()
          .get(&header)
          .and_then(|v| v.to_str().ok())
          .map(|s| s.to_string())
          .unwrap_or_else(|| generator());

        // Inject into request extensions for handler access
        req.extensions_mut().insert(RequestIdValue(id.clone()));

        let mut resp = next.run(req).await;

        // Add to response headers
        if let Ok(val) = HeaderValue::from_str(&id) {
          resp.headers_mut().insert(header, val);
        }

        resp
      })
    }
  }
}
