//! Advertise an alternate HTTP service on HTTP/1 and HTTP/2 responses.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use http::HeaderValue;
use http::Version;
use http::header;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

/// Attach an `Alt-Svc` advertisement without replacing an explicit response value.
///
/// The advertised endpoint must already be running and serve the same origin.
#[derive(Clone)]
pub struct AltSvc {
  value: HeaderValue,
}

impl AltSvc {
  /// Advertise HTTP/3 on the same host at `port` for the given cache lifetime.
  pub fn h3(port: u16, max_age: Duration) -> Self {
    Self::new(
      format!("h3=\":{port}\"; ma={}", max_age.as_secs())
        .parse()
        .expect("valid HTTP/3 advertisement"),
    )
  }

  /// Use a custom advertisement, or `clear` to invalidate cached alternatives.
  pub fn new(value: HeaderValue) -> Self {
    Self { value }
  }
}

impl IntoMiddleware for AltSvc {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send>>
  + Clone
  + Send
  + Sync
  + 'static {
    move |request: Request, next: Next| {
      let advertised = request.version() != Version::HTTP_3;
      let value = self.value.clone();
      Box::pin(async move {
        let mut response = next.run(request).await;
        if advertised {
          response
            .headers_mut()
            .entry(header::ALT_SVC)
            .or_insert(value);
        }
        response
      })
    }
  }
}
