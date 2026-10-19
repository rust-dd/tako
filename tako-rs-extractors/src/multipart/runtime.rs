use std::future::Future;
use std::io;
use std::time::Duration;

pub(super) async fn timeout<F: Future>(duration: Duration, future: F) -> Result<F::Output, ()> {
  #[cfg(feature = "compio")]
  if tokio::runtime::Handle::try_current().is_err() {
    let sleep = std::pin::pin!(send_wrapper::SendWrapper::new(compio::time::sleep(
      duration
    )));
    let work = std::pin::pin!(future);
    return match futures_util::future::select(work, sleep).await {
      futures_util::future::Either::Left((output, _)) => Ok(output),
      futures_util::future::Either::Right(((), _)) => Err(()),
    };
  }
  tokio::time::timeout(duration, future).await.map_err(|_| ())
}

/// The operation owns its resources until it finishes, even if its caller is cancelled.
pub(super) async fn blocking<T: Send + 'static>(
  operation: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> io::Result<T> {
  #[cfg(feature = "compio")]
  if tokio::runtime::Handle::try_current().is_err() {
    // Compio polls and drops the handle on its runtime thread; extractors still require Send.
    return send_wrapper::SendWrapper::new(compio::runtime::spawn_blocking(operation))
      .await
      .map_err(|_| io::Error::other("multipart filesystem task failed"))?;
  }
  tokio::task::spawn_blocking(operation)
    .await
    .map_err(io::Error::other)?
}
