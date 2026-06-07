use std::io;

/// Shared bind-result tracker used by the [`PerThreadShutdown`] coordinator
/// so the parent process can detect "all worker threads failed to bind"
/// (e.g. `SO_REUSEPORT` unavailable on Windows / non-Linux Unix, port already
/// taken) and surface a real error from [`serve_per_thread`](crate::serve_per_thread) instead of
/// silently waiting on Ctrl+C forever and then returning `Ok(())`.
#[derive(Default)]
struct BindStatus {
  /// Number of workers that completed their bind step successfully.
  succeeded: std::sync::atomic::AtomicUsize,
  /// Number of workers that failed their bind step.
  failed: std::sync::atomic::AtomicUsize,
  /// First recorded bind error so the parent can return something
  /// actionable to its caller / supervisor. Plain `std::sync::Mutex` is
  /// fine here — this is a cold path (one write per worker at startup,
  /// one read on shutdown).
  first_err: std::sync::Mutex<Option<io::Error>>,
  /// Wake-up notify so the parent does not have to poll.
  notify: tokio::sync::Notify,
}

/// Shutdown coordinator shared by every worker spawned via [`spawn_per_thread`](crate::spawn_per_thread)
/// (and friends). Workers `select!` against [`Self::notified`] in their accept
/// loop, so triggering [`PerThreadShutdown::trigger`] cleanly exits each
/// worker's `loop { accept }` instead of leaking the OS thread on shutdown.
///
/// Backed by a [`tokio_util::sync::CancellationToken`] so the signal is
/// sticky: workers that register `notified()` after `trigger()` was called
/// still observe the request immediately, fixing the `Notify::notify_waiters`
/// race where late subscribers would miss the shutdown.
///
/// Also carries a private [`BindStatus`] that workers update with the result
/// of their `SO_REUSEPORT` bind so the parent (e.g. [`serve_per_thread`](crate::serve_per_thread)) can
/// fail loudly on "every worker failed to bind" instead of returning Ok(()) —
/// previously the function would await Ctrl+C indefinitely and then claim
/// success even when no listener was up, a false health signal to supervisors.
#[derive(Clone, Default)]
pub struct PerThreadShutdown {
  pub(crate) inner: tokio_util::sync::CancellationToken,
  bind_status: std::sync::Arc<BindStatus>,
}

impl PerThreadShutdown {
  /// Construct an unsignalled shutdown coordinator.
  #[must_use]
  pub fn new() -> Self {
    Self::default()
  }

  /// Notify every worker waiter that it should exit its accept loop.
  /// Idempotent — calling it more than once is a no-op.
  pub fn trigger(&self) {
    self.inner.cancel();
  }

  /// Future a worker awaits to learn that shutdown has been requested.
  pub async fn notified(&self) {
    self.inner.cancelled().await;
  }

  /// Worker hook: report a successful `SO_REUSEPORT` bind.
  pub(crate) fn report_bind_success(&self) {
    self
      .bind_status
      .succeeded
      .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    self.bind_status.notify.notify_waiters();
  }

  /// Worker hook: report a bind failure (with the underlying `io::Error`).
  /// The first error wins for reporting; later errors are dropped after their
  /// `tracing::error!` log.
  pub(crate) fn report_bind_failure(&self, err: io::Error) {
    {
      let mut guard = self.bind_status.first_err.lock().unwrap();
      if guard.is_none() {
        *guard = Some(err);
      }
    }
    self
      .bind_status
      .failed
      .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    self.bind_status.notify.notify_waiters();
  }

  /// Wait until either at least one worker bound successfully, or all
  /// `total` workers reported a bind failure. Returns the first recorded
  /// `io::Error` in the all-failure case so the caller can propagate a real
  /// error instead of pretending the server started.
  pub async fn wait_for_bind_outcome(&self, total: usize) -> io::Result<()> {
    use std::sync::atomic::Ordering;

    loop {
      // Arm the notified future BEFORE reading state so a wake fired
      // between the load and the await is not lost.
      let notified = self.bind_status.notify.notified();
      tokio::pin!(notified);
      notified.as_mut().enable();

      let succ = self.bind_status.succeeded.load(Ordering::SeqCst);
      let fail = self.bind_status.failed.load(Ordering::SeqCst);

      if succ > 0 {
        return Ok(());
      }
      if succ + fail >= total {
        let err = self
          .bind_status
          .first_err
          .lock()
          .unwrap()
          .take()
          .unwrap_or_else(|| {
            io::Error::other(format!("all {total} per-thread workers failed to bind"))
          });
        return Err(err);
      }

      notified.await;
    }
  }
}
