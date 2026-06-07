//! Hyper-on-compio glue that bridges hyper's `Send`-bounded HTTP/2 surface to
//! the single-threaded compio runtime via `send_wrapper`.

#[cfg(feature = "http2")]
use send_wrapper::SendWrapper;

//
// compio is a single-threaded, thread-per-core runtime whose futures are `!Send`.
// hyper's HTTP/2 builder needs an executor to spawn stream handlers and checks
// `Send` at compile time.  Since all spawned futures run on the same thread,
// wrapping them with `SendWrapper` is safe and satisfies the compiler.

/// Wraps a hyper `Service` so its response future type is `Send` via `SendWrapper`.
///
/// This is safe because compio is single-threaded — futures never cross thread
/// boundaries. The `Send` bound is purely a compile-time requirement from hyper's
/// HTTP/2 executor trait, not an actual thread-safety need.
#[cfg(feature = "http2")]
pub(crate) struct ServiceSendWrapper<T>(SendWrapper<T>);

#[cfg(feature = "http2")]
impl<T> ServiceSendWrapper<T> {
  pub(crate) fn new(inner: T) -> Self {
    Self(SendWrapper::new(inner))
  }
}

#[cfg(feature = "http2")]
impl<R, T> hyper::service::Service<R> for ServiceSendWrapper<T>
where
  T: hyper::service::Service<R>,
{
  type Response = T::Response;
  type Error = T::Error;
  type Future = SendWrapper<T::Future>;

  fn call(&self, req: R) -> Self::Future {
    SendWrapper::new(self.0.call(req))
  }
}

/// A hyper executor for compio that accepts `!Send` futures.
///
/// Unlike `cyper_core::CompioExecutor` which requires `F: Send`, this executor
/// accepts any `F: 'static` — but we only use it with `SendWrapper`-wrapped
/// futures, so the `Send` bound is satisfied through the wrapper.
#[cfg(feature = "http2")]
#[derive(Debug, Clone)]
pub(crate) struct CompioH2Executor;

#[cfg(feature = "http2")]
impl<F: std::future::Future<Output = ()> + Send + 'static> hyper::rt::Executor<F>
  for CompioH2Executor
{
  fn execute(&self, fut: F) {
    compio::runtime::spawn(fut).detach();
  }
}

/// A hyper `Timer` implementation backed by `compio::time`.
///
/// Required for HTTP/2 keep-alive pings, stream timeouts, etc.
/// Wraps compio's `!Send` sleep futures in `SendWrapper` to satisfy hyper's bounds.
#[cfg(feature = "http2")]
#[derive(Debug, Clone)]
pub(crate) struct CompioH2Timer;

/// A sleep future that wraps a compio sleep so hyper can hand it across its
/// `Send + Sync` API surface.
///
/// The inner `compio::time::sleep` resolves to `compio_runtime::runtime::time::TimerFuture`,
/// which the upstream crate **explicitly** marks as `!Send + !Sync` (an
/// `assert_not_impl!` in `compio-runtime`): both `poll` and `Drop` reach into
/// the per-thread `Runtime::with_current(...)`, so off-thread access would
/// either panic or corrupt the timer wheel. A bare `unsafe impl Send/Sync`
/// would therefore be unsound — it claims a contract the wrapped future
/// actively rejects.
///
/// `SendWrapper` upholds the contract at runtime: it panics on any deref or
/// drop from a thread other than the one that constructed it, so an
/// accidental cross-thread move becomes a loud panic instead of latent UB.
/// Same pattern as `ServiceSendWrapper` above and `cyper-core::CompioTimer`.
#[cfg(feature = "http2")]
struct CompioSleep(SendWrapper<std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>>);

#[cfg(feature = "http2")]
impl std::future::Future for CompioSleep {
  type Output = ();

  fn poll(
    mut self: std::pin::Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<Self::Output> {
    // SendWrapper's `DerefMut` panics off-thread — the runtime guard for
    // the `Send + Sync` claim. Same thread → cheap atomic load.
    self.0.as_mut().poll(cx)
  }
}

#[cfg(feature = "http2")]
impl Unpin for CompioSleep {}

#[cfg(feature = "http2")]
impl hyper::rt::Sleep for CompioSleep {}

#[cfg(feature = "http2")]
impl hyper::rt::Timer for CompioH2Timer {
  fn sleep(&self, duration: std::time::Duration) -> std::pin::Pin<Box<dyn hyper::rt::Sleep>> {
    Box::pin(CompioSleep(SendWrapper::new(Box::pin(
      compio::time::sleep(duration),
    ))))
  }

  fn sleep_until(&self, deadline: std::time::Instant) -> std::pin::Pin<Box<dyn hyper::rt::Sleep>> {
    Box::pin(CompioSleep(SendWrapper::new(Box::pin(
      compio::time::sleep_until(deadline),
    ))))
  }
}
