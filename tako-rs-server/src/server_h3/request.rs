use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Buf;
use bytes::Bytes;
use bytes::BytesMut;
use h3::quic::BidiStream;
use h3::quic::RecvStream;
use h3::server::RequestStream;
use http::HeaderMap;
use http::Request;
use http_body::Body;
use http_body::Frame;
use tako_rs_core::body::TakoBody;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::conn_info::TlsInfo;
use tako_rs_core::router::Router;
use tako_rs_core::types::BoxError;
use tokio_stream::wrappers::ReceiverStream;

/// Channel buffer for the H3 streaming body.
///
/// Bounds the number of in-flight frames between the QUIC receiver task and the
/// handler so that a slow handler exerts backpressure on the client instead of
/// growing memory unboundedly.
const H3_BODY_CHANNEL_CAPACITY: usize = 8;

/// Tracks live H3 body-forwarder tasks per connection.
///
/// `build_h3_body` spawns a detached forwarder for every accepted stream. The
/// connection drain (`handle_connection`) waits on `request_tasks` for handler
/// completion, but the forwarders run in independent `tokio::spawn` tasks so
/// they were previously not joined before the connection returned. This tracker
/// (counter + Notify) lets the drain wait until every forwarder has finished
/// emitting frames/trailers, bounded by the per-connection grace.
#[derive(Default)]
pub(crate) struct H3BodyTracker {
  pub(crate) active: std::sync::atomic::AtomicUsize,
  pub(crate) drained: tokio::sync::Notify,
}

pub(crate) struct H3BodyGuard {
  tracker: Arc<H3BodyTracker>,
}

impl Drop for H3BodyGuard {
  fn drop(&mut self) {
    if self
      .tracker
      .active
      .fetch_sub(1, std::sync::atomic::Ordering::SeqCst)
      == 1
    {
      self.tracker.drained.notify_waiters();
    }
  }
}

impl H3BodyTracker {
  pub(crate) fn guard(self: &Arc<Self>) -> H3BodyGuard {
    self
      .active
      .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    H3BodyGuard {
      tracker: self.clone(),
    }
  }
}

/// Builds a streaming `TakoBody` backed by an HTTP/3 receive stream.
///
/// Spawns a forwarder task that pulls QUIC chunks via `recv_data`, emits them as
/// `Frame::data`, and then pulls trailers via `recv_trailers` to emit a
/// `Frame::trailers`. The bounded channel provides natural backpressure.
fn build_h3_body<R>(mut recv: RequestStream<R, Bytes>, tracker: Arc<H3BodyTracker>) -> TakoBody
where
  R: RecvStream + Send + 'static,
{
  let (tx, rx) =
    tokio::sync::mpsc::channel::<Result<Frame<Bytes>, BoxError>>(H3_BODY_CHANNEL_CAPACITY);
  let guard = tracker.guard();
  tokio::spawn(async move {
    let _guard = guard;
    loop {
      match recv.recv_data().await {
        Ok(Some(mut chunk)) => {
          let mut buf = BytesMut::with_capacity(chunk.remaining());
          while chunk.has_remaining() {
            let slice = chunk.chunk();
            buf.extend_from_slice(slice);
            let len = slice.len();
            chunk.advance(len);
          }
          if !buf.is_empty() && tx.send(Ok(Frame::data(buf.freeze()))).await.is_err() {
            return;
          }
        }
        Ok(None) => break,
        Err(e) => {
          let _ = tx.send(Err(Box::new(e) as BoxError)).await;
          return;
        }
      }
    }
    match recv.recv_trailers().await {
      Ok(Some(trailers)) => {
        let _ = tx.send(Ok(Frame::trailers(trailers))).await;
      }
      Ok(None) => {}
      Err(e) => {
        let _ = tx.send(Err(Box::new(e) as BoxError)).await;
      }
    }
  });

  TakoBody::from_try_stream(ReceiverStream::new(rx))
}

/// Handles a single HTTP/3 request.
pub(crate) async fn handle_request<S>(
  req: Request<()>,
  stream: RequestStream<S, Bytes>,
  router: Arc<Router>,
  remote_addr: SocketAddr,
  body_tracker: Arc<H3BodyTracker>,
) -> Result<(), BoxError>
where
  S: BidiStream<Bytes> + Send + 'static,
  <S as BidiStream<Bytes>>::SendStream: Send + 'static,
  <S as BidiStream<Bytes>>::RecvStream: Send + 'static,
{
  // Per-request signals fire from inside Router::dispatch.

  // Split into send and recv halves so the handler can stream the body while we
  // hold the send half locally for the response.
  let (mut send_stream, recv_stream) = stream.split();

  // Build request with a streaming body (data + trailers).
  let (parts, ()) = req.into_parts();
  let body = build_h3_body(recv_stream, body_tracker);
  let mut tako_req = Request::from_parts(parts, body);
  tako_req.extensions_mut().insert(remote_addr);
  tako_req.extensions_mut().insert(ConnInfo::h3(
    remote_addr,
    TlsInfo {
      alpn: Some(b"h3".to_vec()),
      sni: None,
      version: Some("TLSv1.3"),
    },
  ));

  // Dispatch through router
  let response = router.dispatch(tako_req).await;

  // Send response head
  let (parts, body) = response.into_parts();
  let resp = http::Response::from_parts(parts, ());
  send_stream.send_response(resp).await?;

  // Stream response body frame by frame; preserve trailers through to send_trailers.
  let mut body = std::pin::pin!(body);
  let mut response_trailers: Option<HeaderMap> = None;
  while let Some(frame_res) = std::future::poll_fn(|cx| body.as_mut().poll_frame(cx)).await {
    match frame_res {
      Ok(frame) => {
        if frame.is_data() {
          if let Ok(data) = frame.into_data()
            && !data.is_empty()
          {
            send_stream.send_data(data).await?;
          }
        } else if frame.is_trailers()
          && let Ok(t) = frame.into_trailers()
        {
          // Last trailer frame wins; HTTP responses are not expected to emit multiple.
          response_trailers = Some(t);
        }
      }
      Err(e) => {
        tracing::error!("HTTP/3 body frame error: {e}");
        break;
      }
    }
  }

  if let Some(trailers) = response_trailers {
    send_stream.send_trailers(trailers).await?;
  } else {
    send_stream.finish().await?;
  }

  Ok(())
}
