use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Buf;
use bytes::Bytes;
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

/// `Send` on Tokio, whose work-stealing runtime may move request tasks between
/// workers; no bound on compio, whose single-threaded runtime keeps every task
/// on the thread that spawned it.
#[cfg(not(feature = "compio"))]
pub(crate) trait RuntimeSend: Send {}
#[cfg(not(feature = "compio"))]
impl<T: Send> RuntimeSend for T {}
#[cfg(feature = "compio")]
pub(crate) trait RuntimeSend {}
#[cfg(feature = "compio")]
impl<T> RuntimeSend for T {}

/// Poll QUIC data and trailers only while the application polls its request body.
fn build_h3_body<R>(recv: RequestStream<R, Bytes>) -> TakoBody
where
  R: RecvStream + RuntimeSend + 'static,
{
  let stream = futures_util::stream::try_unfold(Some(recv), |state| async move {
    let Some(mut recv) = state else {
      return Ok::<_, BoxError>(None);
    };
    match recv.recv_data().await? {
      Some(mut chunk) => {
        let bytes = chunk.copy_to_bytes(chunk.remaining());
        Ok(Some((Frame::data(bytes), Some(recv))))
      }
      None => Ok(
        recv
          .recv_trailers()
          .await?
          .map(|trailers| (Frame::trailers(trailers), None)),
      ),
    }
  });
  #[cfg(not(feature = "compio"))]
  let body = TakoBody::from_try_stream(stream);
  // compio QUIC streams are `!Send`. The runtime keeps them on this thread, and
  // `SendWrapper` panics instead of racing if a handler moves the body away.
  #[cfg(feature = "compio")]
  let body = TakoBody::from_try_stream(send_wrapper::SendWrapper::new(stream));
  body
}

/// Handles a single HTTP/3 request.
pub(crate) async fn handle_request<S>(
  req: Request<()>,
  stream: RequestStream<S, Bytes>,
  router: Arc<Router>,
  remote_addr: SocketAddr,
) -> Result<(), BoxError>
where
  S: BidiStream<Bytes> + RuntimeSend + 'static,
  <S as BidiStream<Bytes>>::SendStream: RuntimeSend + 'static,
  <S as BidiStream<Bytes>>::RecvStream: RuntimeSend + 'static,
{
  // Split into send and recv halves so the handler can stream the body while we
  // hold the send half locally for the response.
  let (mut send_stream, recv_stream) = stream.split();

  let (parts, ()) = req.into_parts();
  let body = build_h3_body(recv_stream);
  let mut tako_req = Request::from_parts(parts, body);
  tako_req.extensions_mut().insert(remote_addr);
  tako_req.extensions_mut().insert(ConnInfo::h3(
    remote_addr,
    TlsInfo {
      alpn: Some(bytes::Bytes::from_static(b"h3")),
      sni: None,
      version: Some("TLSv1.3"),
    },
  ));

  let response = router.dispatch(tako_req).await;

  let (parts, body) = response.into_parts();
  let resp = http::Response::from_parts(parts, ());
  send_stream.send_response(resp).await?;

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
