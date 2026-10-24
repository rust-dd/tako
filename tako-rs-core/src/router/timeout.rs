//! Request-timeout enforcement for the dispatch pipeline.

use std::future::Future;
use std::time::Duration;

use super::Router;
use super::dispatch::empty_status_response;
use crate::types::Request;
use crate::types::Response;

pub(super) async fn run_with_timeout(
  work: impl Future<Output = Response>,
  timeout_duration: Option<Duration>,
) -> Result<Response, ()> {
  let Some(duration) = timeout_duration else {
    return Ok(work.await);
  };
  #[cfg(not(feature = "compio"))]
  {
    tokio::time::timeout(duration, work).await.map_err(|_| ())
  }
  #[cfg(feature = "compio")]
  {
    let sleep = std::pin::pin!(compio::time::sleep(duration));
    let work = std::pin::pin!(work);
    match futures_util::future::select(work, sleep).await {
      futures_util::future::Either::Left((response, _)) => Ok(response),
      futures_util::future::Either::Right(((), _)) => Err(()),
    }
  }
}

impl Router {
  pub(super) fn snapshot_for_timeout(&self, req: &Request) -> Option<Request> {
    self.timeout_fallback.as_ref().map(|_| {
      let mut saved = Request::default();
      *saved.method_mut() = req.method().clone();
      *saved.uri_mut() = req.uri().clone();
      *saved.version_mut() = req.version();
      *saved.headers_mut() = req.headers().clone();
      *saved.extensions_mut() = req.extensions().clone();
      saved
    })
  }

  pub(super) async fn handle_timeout(&self, request: Option<Request>) -> Response {
    if let Some(handler) = &self.timeout_fallback {
      handler.call(request.unwrap_or_default()).await
    } else {
      empty_status_response(self.timeout_status)
    }
  }
}
