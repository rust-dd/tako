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
) -> (ServerHandle, impl Future<Output = ()> + Send + 'static) {
  let shutdown = tokio_util::sync::CancellationToken::new();
  let done = tokio_util::sync::CancellationToken::new();
  let shutdown_for_task = shutdown.clone();
  // `CancellationToken::cancelled()` is sticky — late subscribers still see
  // a cancel that has already fired. Switching from `Notify::notify_waiters`
  // closes the race window where `trigger()` could run before the spawned
  // serve future had a chance to register its waiter.
  let fut = async move {
    shutdown_for_task.cancelled().await;
  };
  (
    ServerHandle {
      shutdown,
      done,
      drain_timeout,
    },
    fut,
  )
}

#[cfg(not(feature = "compio"))]
pub(crate) fn spawn_done<F>(done: tokio_util::sync::CancellationToken, fut: F)
where
  F: Future<Output = ()> + Send + 'static,
{
  tokio::spawn(async move {
    fut.await;
    done.cancel();
  });
}

#[cfg(feature = "compio")]
pub(crate) fn spawn_done_compio<F>(done: tokio_util::sync::CancellationToken, fut: F)
where
  F: Future<Output = ()> + 'static,
{
  compio::runtime::spawn(async move {
    fut.await;
    done.cancel();
  })
  .detach();
}
