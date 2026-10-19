//! Propagates the inbound request id or generates one, exposing it as a [`RequestIdValue`]
//! extension and echoing it in the response header.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::HeaderName;
use http::HeaderValue;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

/// A request ID value that can be extracted from request extensions.
#[derive(Debug, Clone)]
pub struct RequestIdValue(pub String);

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
  /// Uses the `X-Request-ID` header and UUID v4 ids.
  pub fn new() -> Self {
    Self {
      header: HeaderName::from_static("x-request-id"),
      generator: Arc::new(|| uuid::Uuid::new_v4().to_string()),
    }
  }

  /// Panics if `name` is not a valid header name.
  pub fn header_name(mut self, name: &'static str) -> Self {
    self.header = HeaderName::from_bytes(name.as_bytes()).expect("valid request ID header name");
    self
  }

  /// Generated ids that are not valid header values are not echoed in the response.
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
        // The common CDN cap; keeps peers from pinning large ids into logs and headers.
        const MAX_INBOUND_LEN: usize = 256;
        let id = req
          .headers()
          .get(&header)
          .and_then(|v| v.to_str().ok())
          .filter(|s| !s.is_empty() && s.len() <= MAX_INBOUND_LEN)
          .map_or_else(|| generator(), std::string::ToString::to_string);

        req.extensions_mut().insert(RequestIdValue(id.clone()));

        let mut resp = next.run(req).await;

        if let Ok(val) = HeaderValue::from_str(&id) {
          resp.headers_mut().insert(header, val);
        }

        resp
      })
    }
  }
}
