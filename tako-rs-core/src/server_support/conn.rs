//! Per-connection state shared by an HTTP/1 connection, its service, and the
//! response bodies it streams.

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

/// Request activity of one HTTP/1 connection, read by its
/// [`ConnDriver`](super::ConnDriver) to enforce the header deadline.
///
/// Only the connection's own task writes it, so relaxed loads and stores
/// are enough.
#[derive(Debug, Default)]
pub struct ConnActivity {
  requests: AtomicU32,
  busy: AtomicBool,
}

impl ConnActivity {
  /// Records that a request head arrived and its response is pending.
  pub fn request_started(&self) {
    let requests = self.requests.load(Ordering::Relaxed).wrapping_add(1);
    self.requests.store(requests, Ordering::Relaxed);
    self.busy.store(true, Ordering::Relaxed);
  }

  /// Records that the response, including a streamed body, is done.
  pub fn request_finished(&self) {
    self.busy.store(false, Ordering::Relaxed);
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

/// Streamed response body that marks its connection idle once hyper drops it.
struct IdleOnDrop {
  body: TakoBody,
  activity: Arc<ConnActivity>,
}

impl Drop for IdleOnDrop {
  fn drop(&mut self) {
    self.activity.request_finished();
  }
}

impl Body for IdleOnDrop {
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

/// Builds the hyper service for one HTTP/1 TCP connection.
///
/// It records request activity for the header deadline, adds the connection
/// entries on a recycled extension map, and starts the handler of a plain
/// route before hyper polls the response future.
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

/// Frees the connection for the header deadline: at once for a buffered body,
/// when hyper drops it for a streamed one.
fn finish(mut resp: Response, ctx: &ConnCtx) -> Response {
  if resp.body().is_buffered() {
    ctx.activity.request_finished();
  } else {
    let body = std::mem::take(resp.body_mut());
    *resp.body_mut() = TakoBody::new(IdleOnDrop {
      body,
      activity: Arc::clone(&ctx.activity),
    });
  }
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
  fn buffered_response_frees_its_connection_at_once() {
    let ctx = ctx();
    ctx.activity.request_started();
    let resp = finish("ok".into_response(), &ctx);
    assert!(!ctx.activity.is_busy());
    drop(resp);
  }

  #[test]
  fn streaming_response_keeps_its_connection_busy_until_dropped() {
    let ctx = ctx();
    ctx.activity.request_started();
    let stream = futures_util::stream::empty::<Result<Bytes, std::io::Error>>();
    let resp = finish(TakoBody::from_stream(stream).into_response(), &ctx);
    assert!(ctx.activity.is_busy());
    drop(resp);
    assert!(!ctx.activity.is_busy());
  }
}
