use std::future::Future;
use std::time::Duration;

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
  pub(crate) drain_timeout: Duration,
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

  /// Trigger graceful shutdown and await the drain.
  ///
  /// The `_timeout` argument is kept for API symmetry with the original
  /// builder; the actual drain bound is the `drain_timeout` on the
  /// [`ServerConfig`](crate::ServerConfig) that was handed to the builder, enforced inside
  /// `serve_*_with_shutdown`.
  pub async fn shutdown(self, _timeout: Duration) {
    self.shutdown.cancel();
    self.done.cancelled().await;
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
