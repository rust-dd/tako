use std::io;

/// Worker bind results, so the parent can fail when every worker failed to bind instead of
/// waiting for a signal with no listener up.
#[derive(Default)]
struct BindStatus {
  succeeded: std::sync::atomic::AtomicUsize,
  failed: std::sync::atomic::AtomicUsize,
  first_err: std::sync::Mutex<Option<io::Error>>,
  notify: tokio::sync::Notify,
}

/// Stops the workers of [`spawn_per_thread`](crate::spawn_per_thread); the signal is sticky, so
/// a worker that starts waiting after [`trigger`](Self::trigger) still sees it.
#[derive(Clone)]
pub struct PerThreadShutdown {
  pub(crate) inner: tokio_util::sync::CancellationToken,
  bind_status: std::sync::Arc<BindStatus>,
}

impl PerThreadShutdown {
  pub(crate) fn new() -> Self {
    Self {
      inner: tokio_util::sync::CancellationToken::new(),
      bind_status: std::sync::Arc::default(),
    }
  }

  /// Tells every worker to stop accepting and drain; later calls do nothing.
  pub fn trigger(&self) {
    self.inner.cancel();
  }

  /// Resolves once shutdown has been triggered.
  pub async fn notified(&self) {
    self.inner.cancelled().await;
  }

  pub(crate) fn report_bind_success(&self) {
    self
      .bind_status
      .succeeded
      .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    self.bind_status.notify.notify_waiters();
  }

  /// Records a worker's bind failure; only the first error is kept for the caller.
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

  /// Waits until one worker has bound, or returns the first bind error once all `total` failed.
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
