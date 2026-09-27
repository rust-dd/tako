use std::future::Future;
use std::time::Duration;

use super::handle::ServerHandle;

/// ALPN list used by TCP-based TLS spawn paths. Mirrors the per-feature
/// negotiation already done in `server_tls{,_compio}::run`.
#[cfg(feature = "tls")]
#[inline]
pub(crate) fn tls_alpn_for_tcp() -> Vec<Vec<u8>> {
  #[cfg(feature = "http2")]
  {
    vec![b"h2".to_vec(), b"http/1.1".to_vec()]
  }
  #[cfg(not(feature = "http2"))]
  {
    vec![b"http/1.1".to_vec()]
  }
}

pub(crate) fn make_handle(
  drain_timeout: Duration,
  local_addr: Option<std::net::SocketAddr>,
) -> (ServerHandle, impl Future<Output = ()> + Send + 'static) {
  let shutdown = tokio_util::sync::CancellationToken::new();
  let shutdown_for_task = shutdown.clone();
  (
    ServerHandle {
      shutdown,
      done: tokio_util::sync::CancellationToken::new(),
      abort: tokio_util::sync::CancellationToken::new(),
      drain_timeout,
      result: std::sync::Arc::default(),
      local_addr,
    },
    async move { shutdown_for_task.cancelled().await },
  )
}

pub(crate) fn failed_handle(
  error: tako_rs_core::types::BoxError,
  drain_timeout: Duration,
) -> ServerHandle {
  let (handle, _) = make_handle(drain_timeout, None);
  *handle.result.lock().unwrap() = Some(Err(error.into()));
  handle.done.cancel();
  handle
}

#[cfg(not(feature = "compio"))]
pub(crate) fn spawn_done<F>(handle: &ServerHandle, fut: F)
where
  F: Future<Output = Result<(), tako_rs_core::types::BoxError>> + Send + 'static,
{
  tokio::spawn(run_until_aborted(handle, fut));
}

#[cfg(feature = "compio")]
pub(crate) fn spawn_done_compio<F>(handle: &ServerHandle, fut: F)
where
  F: Future<Output = Result<(), tako_rs_core::types::BoxError>> + 'static,
{
  compio::runtime::spawn(run_until_aborted(handle, fut)).detach();
}

fn run_until_aborted<F>(handle: &ServerHandle, fut: F) -> impl Future<Output = ()> + use<F>
where
  F: Future<Output = Result<(), tako_rs_core::types::BoxError>>,
{
  use futures_util::FutureExt;
  let mut completion = Completion {
    done: handle.done.clone(),
    result: handle.result.clone(),
    outcome: None,
  };
  let abort = handle.abort.clone();
  async move {
    let outcome = {
      let task = std::pin::pin!(std::panic::AssertUnwindSafe(fut).catch_unwind());
      let stop = std::pin::pin!(abort.cancelled());
      match futures_util::future::select(task, stop).await {
        futures_util::future::Either::Left((Ok(result), _)) => result.map_err(Into::into),
        futures_util::future::Either::Left((Err(_), _)) => Err(super::handle::ServerError(
          std::sync::Arc::new(std::io::Error::other("server task panicked")),
        )),
        futures_util::future::Either::Right(_) => Err(super::handle::ServerError(
          std::sync::Arc::new(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "server shutdown deadline exceeded",
          )),
        )),
      }
    };
    completion.outcome = Some(outcome);
    drop(completion);
  }
}

struct Completion {
  done: tokio_util::sync::CancellationToken,
  result: std::sync::Arc<std::sync::Mutex<Option<Result<(), super::handle::ServerError>>>>,
  outcome: Option<Result<(), super::handle::ServerError>>,
}

impl Drop for Completion {
  fn drop(&mut self) {
    let outcome = self.outcome.take().unwrap_or_else(|| {
      Err(super::handle::ServerError(std::sync::Arc::new(
        std::io::Error::new(std::io::ErrorKind::Interrupted, "server task cancelled"),
      )))
    });
    *self
      .result
      .lock()
      .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(outcome);
    self.done.cancel();
  }
}
