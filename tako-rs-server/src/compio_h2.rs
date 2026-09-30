//! Hyper-on-compio glue that bridges hyper's `Send`-bounded HTTP/2 surface to
//! the single-threaded compio runtime via `send_wrapper`. Shared by the TLS
//! (ALPN `h2`) and cleartext h2c servers.
//!
//! # `send_wrapper` invariant — hard contract
//!
//! Hyper's HTTP/2 server builder requires `Send` on the response future and
//! the executor it hands work to. The compio runtime is **single-threaded
//! per core**: every future created by `compio::runtime::spawn` is `!Send`
//! and is polled exclusively on the runtime thread that produced it.
//!
//! * `ServiceSendWrapper` wraps the per-connection hyper service and its
//!   response future in `SendWrapper`, satisfying hyper's bound at the type
//!   level.
//! * `CompioH2Executor` re-`spawn`s those `Send`-claimed futures back onto
//!   the same compio runtime thread.
//! * `CompioH2Timer` wraps `compio::time::sleep` similarly so HTTP/2
//!   keep-alive timers can be handed to hyper.
//!
//! **The soundness of this pattern depends on the wrapped values never
//! crossing a thread boundary at runtime.** That holds because:
//!
//! 1. The compio runtime is per-thread — futures are pinned to the thread
//!    that called `spawn`, and there is no cross-thread work-stealing.
//! 2. `SendWrapper<T>` panics on drop or deref from any thread other than
//!    the one that constructed it, so an accidental cross-thread move
//!    becomes a loud panic instead of UB.
//! 3. We never construct a `SendWrapper` outside of a compio runtime task,
//!    and we never hand the wrapper to a multi-threaded tokio runtime.
//!
//! The `Send` claim made by `SendWrapper<T>` is therefore **per-runtime, not
//! global**. Anyone moving these types out of the compio path (e.g. mixing
//! a tokio executor in front of `ServiceSendWrapper`) breaks the invariant.

use hyper::server::conn::http2;
use send_wrapper::SendWrapper;

/// Wraps a hyper `Service` so its response future type is `Send` via `SendWrapper`.
///
/// This is safe because compio is single-threaded — futures never cross thread
/// boundaries. The `Send` bound is purely a compile-time requirement from hyper's
/// HTTP/2 executor trait, not an actual thread-safety need.
pub(crate) struct ServiceSendWrapper<T>(SendWrapper<T>);

impl<T> ServiceSendWrapper<T> {
  pub(crate) fn new(inner: T) -> Self {
    Self(SendWrapper::new(inner))
  }
}

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
#[derive(Debug, Clone)]
pub(crate) struct CompioH2Executor;

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
struct CompioSleep(SendWrapper<std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>>);

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

impl Unpin for CompioSleep {}

impl hyper::rt::Sleep for CompioSleep {}

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

/// HTTP/2 limits copied out of [`crate::ServerConfig`] once per server.
#[derive(Clone, Copy)]
pub(crate) struct H2Settings {
  max_concurrent_streams: u32,
  max_header_list_size: u32,
  max_send_buf_size: usize,
  max_pending_accept_reset_streams: usize,
  keep_alive_interval: Option<std::time::Duration>,
}

impl From<&crate::ServerConfig> for H2Settings {
  fn from(config: &crate::ServerConfig) -> Self {
    Self {
      max_concurrent_streams: config.h2_max_concurrent_streams,
      max_header_list_size: config.h2_max_header_list_size,
      max_send_buf_size: config.h2_max_send_buf_size,
      max_pending_accept_reset_streams: config.h2_max_pending_accept_reset_streams,
      keep_alive_interval: config.h2_keep_alive_interval,
    }
  }
}

impl H2Settings {
  pub(crate) fn builder(self) -> http2::Builder<CompioH2Executor> {
    let mut builder = http2::Builder::new(CompioH2Executor);
    builder
      .timer(CompioH2Timer)
      .max_concurrent_streams(self.max_concurrent_streams)
      .max_header_list_size(self.max_header_list_size)
      .max_send_buf_size(self.max_send_buf_size)
      .max_pending_accept_reset_streams(self.max_pending_accept_reset_streams);
    if let Some(interval) = self.keep_alive_interval {
      builder.keep_alive_interval(Some(interval));
    }
    builder
  }
}
