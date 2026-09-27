use std::future::Future;
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

/// Background-task handle returned by every `spawn_*` method.
///
/// Drop semantics: dropping the handle does **not** stop the server. Call
/// [`ServerHandle::shutdown`] (or [`ServerHandle::trigger`] + `.join().await`)
/// so the drain logic in the underlying `serve_*_with_shutdown` runs.
///
/// Runtime-agnostic — the `done` signal is fired by an `async` wrapper around
/// the underlying `serve_*` future, so the same `ServerHandle` works whether
/// the spawned task lives on the tokio runtime or the compio runtime.
pub struct ServerHandle {
  pub(crate) shutdown: tokio_util::sync::CancellationToken,
  pub(crate) done: tokio_util::sync::CancellationToken,
  pub(crate) abort: tokio_util::sync::CancellationToken,
  pub(crate) drain_timeout: Duration,
  pub(crate) result: Arc<Mutex<Option<Result<(), ServerError>>>>,
  pub(crate) local_addr: Option<SocketAddr>,
}

impl ServerHandle {
  /// Trigger graceful shutdown without awaiting completion.
  pub fn trigger(&self) {
    self.shutdown.cancel();
  }

  /// Await the spawned task's completion (without triggering shutdown).
  ///
  /// Returns when the underlying `serve_*` future resolves — typically
  /// because [`ServerHandle::trigger`] / [`ServerHandle::shutdown`] was called
  /// or because the listener errored fatally.
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

  /// Returns the bound IP address, including the chosen port for port-zero binds.
  /// Unix-domain and vsock listeners return `None`.
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

  /// Returns the drain timeout the underlying `serve_*` will honor.
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

/// Convenience: await `signal_a` *or* `signal_b`, whichever fires first.
pub async fn either<A, B>(a: A, b: B)
where
  A: Future<Output = ()>,
  B: Future<Output = ()>,
{
  use futures_util::future::Either;
  let a = std::pin::pin!(a);
  let b = std::pin::pin!(b);
  match futures_util::future::select(a, b).await {
    Either::Left(_) | Either::Right(_) => {}
  }
}
