use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

/// A clonable server startup or background-task failure.
#[derive(Debug, Clone)]
pub struct ServerError(pub(crate) Arc<dyn std::error::Error + Send + Sync>);

impl std::fmt::Display for ServerError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    self.0.fmt(f)
  }
}

impl std::error::Error for ServerError {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    Some(self.0.as_ref())
  }
}

impl From<tako_rs_core::types::BoxError> for ServerError {
  fn from(error: tako_rs_core::types::BoxError) -> Self {
    Self(Arc::from(error))
  }
}

/// Handle to a spawned server on either runtime. Dropping it does not stop the server; call
/// [`ServerHandle::shutdown`], or [`ServerHandle::trigger`] and then `join`, to drain it.
pub struct ServerHandle {
  pub(crate) shutdown: tokio_util::sync::CancellationToken,
  pub(crate) done: tokio_util::sync::CancellationToken,
  pub(crate) abort: tokio_util::sync::CancellationToken,
  pub(crate) drain_timeout: Duration,
  pub(crate) result: Arc<Mutex<Option<Result<(), ServerError>>>>,
  pub(crate) local_addr: Option<SocketAddr>,
}

impl ServerHandle {
  /// Stops accepting and starts draining without waiting for it.
  pub fn trigger(&self) {
    self.shutdown.cancel();
  }

  /// Waits until the server stops, after a shutdown or a fatal listener error, without
  /// triggering shutdown.
  pub async fn join(&self) {
    self.done.cancelled().await;
  }

  /// Awaits completion and returns startup, I/O, panic, or forced-abort errors.
  pub async fn result(&self) -> Result<(), ServerError> {
    self.join().await;
    self
      .result
      .lock()
      .unwrap_or_else(std::sync::PoisonError::into_inner)
      .as_ref()
      .expect("completed task has a result")
      .clone()
  }

  /// The bound address, including the port chosen for a port-zero bind; `None` for Unix and vsock.
  pub fn local_addr(&self) -> Option<SocketAddr> {
    self.local_addr
  }

  /// Triggers shutdown and waits at most the smaller of `timeout` and the
  /// configured drain timeout before aborting remaining server work.
  pub async fn shutdown(self, timeout: Duration) {
    self.trigger();
    let deadline = async {
      #[cfg(not(feature = "compio"))]
      tokio::time::sleep(timeout.min(self.drain_timeout)).await;
      #[cfg(feature = "compio")]
      compio::time::sleep(timeout.min(self.drain_timeout)).await;
    };
    let finished = std::pin::pin!(self.done.cancelled());
    let deadline = std::pin::pin!(deadline);
    if let futures_util::future::Either::Right(_) =
      futures_util::future::select(finished, deadline).await
    {
      self.abort.cancel();
      self.done.cancelled().await;
    }
  }

  /// Waits for Ctrl+C or SIGTERM, then drains the server.
  pub async fn shutdown_on_signal(self) -> std::io::Result<()> {
    tako_rs_core::server_support::shutdown_signal().await?;
    let timeout = self.drain_timeout;
    self.shutdown(timeout).await;
    Ok(())
  }

  /// `ServerConfig::drain_timeout`, which also caps [`ServerHandle::shutdown`].
  #[inline]
  pub fn drain_timeout(&self) -> Duration {
    self.drain_timeout
  }
}

impl std::fmt::Debug for ServerHandle {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("ServerHandle")
      .field("drain_timeout", &self.drain_timeout)
      .finish_non_exhaustive()
  }
}
