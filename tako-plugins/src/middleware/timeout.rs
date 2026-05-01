//! Per-request timeout middleware.
//!
//! Aborts the inner middleware chain when a configurable deadline is exceeded
//! and returns `503 Service Unavailable` (or a caller-supplied status). The
//! timer also covers any work the handler is still doing — `tokio::time::timeout`
//! drops the inner future, which cancels in-flight async work tied to the
//! request future tree.
//!
//! For per-route timeouts that bypass the middleware chain entirely, use
//! [`Route::timeout`](tako_core::route::Route::timeout) instead — this
//! middleware exists for cases where the deadline is dynamic (per-tenant,
//! per-IP, …) or composes with other middleware (e.g. retry).
//!
//! # Examples
//!
//! ```rust,ignore
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
use tako_core::body::TakoBody;
use tako_core::middleware::IntoMiddleware;
use tako_core::middleware::Next;
use tako_core::types::Request;
use tako_core::types::Response;

/// Per-request timeout middleware configuration.
pub struct Timeout {
  duration: Duration,
  status: StatusCode,
  dynamic: Option<Arc<dyn Fn(&Request) -> Option<Duration> + Send + Sync + 'static>>,
}

impl Timeout {
  /// Creates a timeout middleware with a static deadline.
  pub fn new(duration: Duration) -> Self {
    Self {
      duration,
      status: StatusCode::SERVICE_UNAVAILABLE,
      dynamic: None,
    }
  }

  /// Sets the response status used when the deadline elapses. Default: 503.
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
        let deadline = dynamic
          .as_ref()
          .and_then(|f| f(&req))
          .or(Some(default_duration));

        let fut = next.run(req);
        match deadline {
          Some(d) => {
            #[cfg(not(feature = "compio"))]
            {
              match tokio::time::timeout(d, fut).await {
                Ok(resp) => resp,
                Err(_) => http::Response::builder()
                  .status(status)
                  .body(TakoBody::empty())
                  .expect("valid timeout response"),
              }
            }
            #[cfg(feature = "compio")]
            {
              use futures_util::future::Either;
              let timer = compio::time::sleep(d);
              futures_util::pin_mut!(timer);
              match futures_util::future::select(std::pin::pin!(fut), timer).await {
                Either::Left((resp, _)) => resp,
                Either::Right((_, _)) => http::Response::builder()
                  .status(status)
                  .body(TakoBody::empty())
                  .expect("valid timeout response"),
              }
            }
          }
          None => fut.await,
        }
      })
    }
  }
}
