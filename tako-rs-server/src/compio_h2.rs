//! Runs hyper's `Send`-bound HTTP/2 server on compio. `SendWrapper` is sound here only because
//! compio never moves a task off its thread, and it panics instead of racing if one is moved.

use hyper::server::conn::http2;
use send_wrapper::SendWrapper;

/// Makes a service and its response future `Send` for hyper's HTTP/2 executor bound.
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

/// Spawns hyper's HTTP/2 tasks, made `Send` by [`ServiceSendWrapper`], on the current thread.
#[derive(Debug, Clone)]
pub(crate) struct CompioH2Executor;

impl<F: std::future::Future<Output = ()> + Send + 'static> hyper::rt::Executor<F>
  for CompioH2Executor
{
  fn execute(&self, fut: F) {
    compio::runtime::spawn(fut).detach();
  }
}

/// `compio::time` timer for HTTP/2 keep-alive pings and timeouts.
#[derive(Debug, Clone)]
pub(crate) struct CompioH2Timer;

/// compio's timer future is `!Send + !Sync` because it reaches into the thread's runtime, so it
/// goes through `SendWrapper` rather than an `unsafe impl`.
struct CompioSleep(SendWrapper<std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>>);

impl std::future::Future for CompioSleep {
  type Output = ();

  fn poll(
    mut self: std::pin::Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<Self::Output> {
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
