//! Request deadlines answered with 503, dropping the handler future; Tokio only, so use
//! [`Route::timeout`](tako_rs_core::route::Route::timeout) under `compio`.
//!
//! # Examples
//!
//! ```rust
//! use std::time::Duration;
//! use tako::middleware::timeout::Timeout;
//! use tako::middleware::IntoMiddleware;
//!
//! let mw = Timeout::new(Duration::from_secs(30)).into_middleware();
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use http::StatusCode;
use tako_rs_core::body::TakoBody;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

/// Per-request override closure for [`Timeout`].
pub type TimeoutDynamicFn = Arc<dyn Fn(&Request) -> Option<Duration> + Send + Sync + 'static>;

pub struct Timeout {
  duration: Duration,
  status: StatusCode,
  dynamic: Option<TimeoutDynamicFn>,
}

impl Timeout {
  pub fn new(duration: Duration) -> Self {
    Self {
      duration,
      status: StatusCode::SERVICE_UNAVAILABLE,
      dynamic: None,
    }
  }

  /// Defaults to 503.
  pub fn status(mut self, status: StatusCode) -> Self {
    self.status = status;
    self
  }

  /// Computes the deadline per request. Returning `None` disables the timeout
  /// for that request.
  pub fn dynamic<F>(mut self, f: F) -> Self
  where
    F: Fn(&Request) -> Option<Duration> + Send + Sync + 'static,
  {
    self.dynamic = Some(Arc::new(f));
    self
  }
}

impl IntoMiddleware for Timeout {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let default_duration = self.duration;
    let status = self.status;
    let dynamic = self.dynamic;

    move |req: Request, next: Next| {
      let dynamic = dynamic.clone();
      Box::pin(async move {
        // A dynamic fn's `None` disables the timeout rather than falling back to the default.
        let deadline = match dynamic.as_ref() {
          Some(f) => f(&req),
          None => Some(default_duration),
        };

        let fut = next.run(req);
        match deadline {
          Some(d) => match tokio::time::timeout(d, fut).await {
            Ok(resp) => resp,
            Err(_) => http::Response::builder()
              .status(status)
              .body(TakoBody::empty())
              .expect("valid timeout response"),
          },
          None => fut.await,
        }
      })
    }
  }
}
