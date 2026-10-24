use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;

use bytes::Bytes;
use futures_util::future::Either;
use http_body::Body;
use http_body::Frame;
use http_body::SizeHint;
use hyper::body::Incoming;
use hyper::service::Service;

use crate::body::TakoBody;
use crate::router::Dispatch;
use crate::router::Router;
use crate::types::BoxError;
use crate::types::Request;
use crate::types::Response;

/// Request activity behind the [`ConnDriver`](super::ConnDriver) header deadline; only the
/// connection's own task writes it, so relaxed ordering is enough.
#[derive(Debug, Default)]
pub struct ConnActivity {
  requests: AtomicU32,
  busy: AtomicBool,
  body_finished: AtomicBool,
}

impl ConnActivity {
  /// Records that a request head arrived and its response is pending.
  pub fn request_started(&self) {
    let requests = self.requests.load(Ordering::Relaxed).wrapping_add(1);
    self.requests.store(requests, Ordering::Relaxed);
    self.body_finished.store(false, Ordering::Relaxed);
    self.busy.store(true, Ordering::Relaxed);
  }

  /// Records that Hyper consumed the response body, which may still be buffered for writing.
  pub fn request_finished(&self) {
    self.body_finished.store(true, Ordering::Relaxed);
  }

  /// Records that all bytes buffered by Hyper have reached the underlying IO.
  pub fn response_flushed(&self) {
    if self.body_finished.load(Ordering::Relaxed) {
      self.busy.store(false, Ordering::Relaxed);
    }
  }

  /// Whether a request is in flight or its response body still streams.
  pub fn is_busy(&self) -> bool {
    self.busy.load(Ordering::Relaxed)
  }

  pub(super) fn requests(&self) -> u32 {
    self.requests.load(Ordering::Relaxed)
  }
}

/// What one HTTP/1 TCP connection shares with its requests.
pub struct ConnCtx {
  router: Arc<Router>,
  peer: SocketAddr,
  /// Request activity the header deadline watches; shared with the bodies of
  /// streamed responses.
  pub activity: Arc<ConnActivity>,
}

impl ConnCtx {
  pub fn new(router: Arc<Router>, peer: SocketAddr) -> Self {
    Self {
      router,
      peer,
      activity: Arc::default(),
    }
  }
}

/// Response body that keeps the deadline suspended until Hyper consumes and flushes it.
struct WritingOnDrop {
  body: TakoBody,
  activity: Arc<ConnActivity>,
}

impl Drop for WritingOnDrop {
  fn drop(&mut self) {
    self.activity.request_finished();
  }
}

impl Body for WritingOnDrop {
  type Data = Bytes;
  type Error = BoxError;

  #[inline]
  fn poll_frame(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<Option<Result<Frame<Bytes>, BoxError>>> {
    Pin::new(&mut self.get_mut().body).poll_frame(cx)
  }

  #[inline]
  fn size_hint(&self) -> SizeHint {
    self.body.size_hint()
  }

  #[inline]
  fn is_end_stream(&self) -> bool {
    self.body.is_end_stream()
  }
}

/// Hyper service for one HTTP/1 connection: records activity for the header deadline and
/// starts a plain route's handler before hyper polls the response future.
pub fn tcp_service<H>(
  ctx: H,
) -> impl Service<
  hyper::Request<Incoming>,
  Response = Response,
  Error = Infallible,
  Future = impl Future<Output = Result<Response, Infallible>>,
>
where
  H: Deref<Target = ConnCtx> + Clone + 'static,
{
  hyper::service::service_fn(move |req: hyper::Request<Incoming>| {
    let ctx = ctx.clone();
    ctx.activity.request_started();
    let req = req.map(TakoBody::incoming);
    // Boxing the full pipeline keeps this future small: hyper moves it into
    // place on every request, and the full pipeline's state is large.
    let pending = match ctx.router.begin(req, Some(ctx.peer)) {
      Dispatch::Handler(handler) => Either::Left(handler),
      Dispatch::Full(req) => Either::Right(Box::pin(dispatch_full(ctx.clone(), req))),
    };
    async move { Ok::<_, Infallible>(finish(pending.await, &ctx)) }
  })
}

async fn dispatch_full<H: Deref<Target = ConnCtx>>(ctx: H, req: Request) -> Response {
  ctx.router.dispatch_full(req).await
}

/// Keeps buffered and streamed responses busy until their final socket flush.
fn finish(mut resp: Response, ctx: &ConnCtx) -> Response {
  let body = std::mem::take(resp.body_mut());
  *resp.body_mut() = TakoBody::new(WritingOnDrop {
    body,
    activity: Arc::clone(&ctx.activity),
  });
  resp
}

#[cfg(test)]
mod tests {
  use std::net::SocketAddr;
  use std::sync::Arc;

  use bytes::Bytes;

  use super::ConnCtx;
  use super::finish;
  use crate::body::TakoBody;
  use crate::responder::Responder;
  use crate::router::Router;

  fn peer(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
  }

  fn ctx() -> Arc<ConnCtx> {
    Arc::new(ConnCtx::new(Arc::new(Router::new()), peer(4000)))
  }

  #[test]
  fn buffered_response_keeps_its_connection_busy_until_flushed() {
    let ctx = ctx();
    ctx.activity.request_started();
    let resp = finish("ok".into_response(), &ctx);
    assert!(ctx.activity.is_busy());
    ctx.activity.response_flushed();
    assert!(ctx.activity.is_busy());
    drop(resp);
    assert!(ctx.activity.is_busy());
    ctx.activity.response_flushed();
    assert!(!ctx.activity.is_busy());
  }

  #[test]
  fn streaming_response_keeps_its_connection_busy_until_flushed() {
    let ctx = ctx();
    ctx.activity.request_started();
    let stream = futures_util::stream::empty::<Result<Bytes, std::io::Error>>();
    let resp = finish(TakoBody::from_stream(stream).into_response(), &ctx);
    assert!(ctx.activity.is_busy());
    drop(resp);
    assert!(ctx.activity.is_busy());
    ctx.activity.response_flushed();
    assert!(!ctx.activity.is_busy());
  }
}
