//! Request-timeout enforcement for the dispatch pipeline.

use std::time::Duration;

use http::StatusCode;

use super::Router;
use super::dispatch::empty_status_response;
use crate::middleware::Next;
use crate::types::Request;
use crate::types::Response;

impl Router {
  /// Executes the middleware chain with an optional timeout.
  ///
  /// If a timeout is specified and exceeded, the timeout fallback handler
  /// is invoked or a default 408 Request Timeout response is returned.
  pub(super) async fn run_with_timeout(
    &self,
    req: Request,
    next: Next,
    timeout_duration: Option<Duration>,
  ) -> Response {
    match timeout_duration {
      Some(duration) => {
        #[cfg(not(feature = "compio"))]
        {
          match tokio::time::timeout(duration, next.run(req)).await {
            Ok(response) => response,
            Err(_elapsed) => self.handle_timeout().await,
          }
        }
        #[cfg(feature = "compio")]
        {
          let sleep = std::pin::pin!(compio::time::sleep(duration));
          let work = std::pin::pin!(next.run(req));
          match futures_util::future::select(work, sleep).await {
            futures_util::future::Either::Left((response, _)) => response,
            futures_util::future::Either::Right(((), _)) => self.handle_timeout().await,
          }
        }
      }
      None => next.run(req).await,
    }
  }

  /// Returns the timeout response using the fallback handler or a default 408.
  async fn handle_timeout(&self) -> Response {
    if let Some(handler) = &self.timeout_fallback {
      handler.call(Request::default()).await
    } else {
      empty_status_response(StatusCode::REQUEST_TIMEOUT)
    }
  }
}
