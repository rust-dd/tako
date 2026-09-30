//! The server side of one WebTransport session on an h3 connection.
//!
//! Adapted from `h3-webtransport` 0.1.2 (<https://github.com/hyperium/h3>,
//! MIT License, Copyright (c) 2020 h3 authors). Unlike upstream, it hands the
//! CONNECT stream back to the caller, which reads the client's close and
//! sends its own.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;
use std::task::Poll;

use bytes::Buf;
use bytes::Bytes;
use futures_util::future::poll_fn;
use h3::ConnectionState;
use h3::SharedState;
use h3::error::Code;
use h3::error::ConnectionError;
use h3::error::StreamError;
use h3::error::connection_error_creators::CloseStream;
use h3::error::internal_error::InternalConnectionError;
use h3::frame::FrameStream;
use h3::proto::frame::Frame;
use h3::quic;
use h3::quic::OpenStreams;
use h3::quic::WriteBuf;
use h3::server::Connection;
use h3::server::RequestStream;
use h3::stream::BidiStreamHeader;
use h3::stream::BufRecvStream;
use h3::stream::UniStreamHeader;
use h3::webtransport::SessionId;
use h3_datagram::datagram_handler::DatagramReader;
use h3_datagram::datagram_handler::DatagramSender;
use h3_datagram::datagram_handler::HandleDatagramsExt;
use h3_datagram::quic_traits::DatagramConnectionExt;
use http::Request;
use http::Response;
use http::StatusCode;

use super::stream::BidiStream;
use super::stream::RecvStream;
use super::stream::SendStream;
use crate::h3_common::connection::QuicConnection;
use crate::h3_common::sessions::ServerStream;

type Bidi = <QuicConnection as OpenStreams<Bytes>>::BidiStream;
pub(super) type ConnectSend = RequestStream<<Bidi as quic::BidiStream<Bytes>>::SendStream, Bytes>;
pub(super) type ConnectRecv = RequestStream<<Bidi as quic::BidiStream<Bytes>>::RecvStream, Bytes>;
type Reader = DatagramReader<<QuicConnection as DatagramConnectionExt<Bytes>>::RecvDatagramHandler>;
type Sender =
  DatagramSender<<QuicConnection as DatagramConnectionExt<Bytes>>::SendDatagramHandler, Bytes>;

pub(super) struct Driver {
  id: SessionId,
  conn: Mutex<Connection<QuicConnection, Bytes>>,
  opener: Mutex<<QuicConnection as quic::Connection<Bytes>>::OpenStreams>,
  shared: Arc<SharedState>,
}

impl ConnectionState for Driver {
  fn shared_state(&self) -> &SharedState {
    &self.shared
  }
}

impl CloseStream for Driver {}

impl Driver {
  /// Answers the CONNECT request with `200 OK` and takes over `conn`.
  pub(super) async fn accept(
    mut stream: ServerStream,
    mut conn: Connection<QuicConnection, Bytes>,
  ) -> Result<(Self, ConnectSend, ConnectRecv), StreamError> {
    let shared = conn.inner.shared.clone();
    let peer = shared.settings();
    let missing = if peer.enable_webtransport() {
      (!peer.enable_datagram()).then_some("datagrams are not supported by the client")
    } else {
      Some("webtransport is not supported by the client")
    };
    if let Some(missing) = missing {
      let error = InternalConnectionError::new(Code::H3_SETTINGS_ERROR, missing.to_owned());
      return Err(StreamError::ConnectionError(
        conn.inner.handle_connection_error(error),
      ));
    }

    // Chrome needs this header, which names the draft the server speaks.
    let response = Response::builder()
      .status(StatusCode::OK)
      .header("sec-webtransport-http3-draft", "draft02")
      .body(())
      .expect("a static response is valid");
    stream.send_response(response).await?;

    let id = stream.send_id().into();
    let opener = Mutex::new(quic::Connection::<Bytes>::opener(&conn.inner.conn));
    let (send, recv) = stream.split();
    let driver = Self {
      id,
      conn: Mutex::new(conn),
      opener,
      shared,
    };
    Ok((driver, send, recv))
  }

  pub(super) fn id(&self) -> SessionId {
    self.id
  }

  fn conn(&self) -> MutexGuard<'_, Connection<QuicConnection, Bytes>> {
    self.conn.lock().unwrap_or_else(PoisonError::into_inner)
  }

  /// Waits for the next bidirectional WebTransport stream; `None` once the
  /// connection closes. HTTP/3 requests that arrive meanwhile go to
  /// `on_request`.
  pub(super) async fn accept_bi(
    &self,
    mut on_request: impl FnMut(Request<()>, ServerStream),
  ) -> Result<Option<(SessionId, BidiStream)>, StreamError> {
    loop {
      let stream = match poll_fn(|cx| self.conn().poll_accept_request_stream(cx)).await {
        Ok(Some(stream)) => FrameStream::new(BufRecvStream::new(stream)),
        Ok(None) => return Ok(None),
        Err(error) => return Err(StreamError::ConnectionError(error)),
      };
      let mut resolver = self.conn().create_resolver(stream);
      // The first frame tells a WebTransport stream from a request.
      match poll_fn(|cx| resolver.frame_stream.poll_next(cx)).await {
        // Closed before its first frame; wait for the next stream.
        Ok(None) => {}
        Ok(Some(Frame::WebTransportStream(id))) => {
          let stream = BidiStream::new(resolver.frame_stream.into_inner());
          return Ok(Some((id, stream)));
        }
        frame => {
          let (req, stream) = resolver.accept_with_frame(frame)?.resolve().await?;
          on_request(req, stream);
        }
      }
    }
  }

  /// Waits for the next unidirectional WebTransport stream.
  pub(super) async fn accept_uni(&self) -> Result<(SessionId, RecvStream), ConnectionError> {
    poll_fn(|cx| {
      let mut conn = self.conn();
      conn.inner.poll_accept_recv(cx)?;
      match conn.inner.accepted_streams_mut().wt_uni_streams.pop() {
        Some((id, stream)) => Poll::Ready(Ok((id, RecvStream::new(stream)))),
        None => Poll::Pending,
      }
    })
    .await
  }

  pub(super) async fn open_bi(&self) -> Result<BidiStream, StreamError> {
    let stream = poll_fn(|cx| {
      self
        .opener
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .poll_open_bidi(cx)
    })
    .await
    .map_err(|error| self.handle_quic_stream_error(error))?;
    let mut stream = BidiStream::new(BufRecvStream::new(stream));
    let mut header: WriteBuf<&'static [u8]> =
      WriteBuf::from(BidiStreamHeader::WebTransportBidi(self.id));
    while header.has_remaining() {
      poll_fn(|cx| stream.poll_send_header(cx, &mut header))
        .await
        .map_err(|error| self.handle_quic_stream_error(error))?;
    }
    Ok(stream)
  }

  pub(super) async fn open_uni(&self) -> Result<SendStream, StreamError> {
    let stream = poll_fn(|cx| {
      self
        .opener
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .poll_open_send(cx)
    })
    .await
    .map_err(|error| self.handle_quic_stream_error(error))?;
    let mut stream = SendStream::new(BufRecvStream::new(stream));
    let mut header: WriteBuf<&'static [u8]> =
      WriteBuf::from(UniStreamHeader::WebTransportUni(self.id));
    while header.has_remaining() {
      poll_fn(|cx| stream.poll_send_header(cx, &mut header))
        .await
        .map_err(|error| self.handle_quic_stream_error(error))?;
    }
    Ok(stream)
  }

  pub(super) fn datagram_reader(&self) -> Reader {
    self.conn().get_datagram_reader()
  }

  pub(super) fn datagram_sender(&self) -> Sender {
    self.conn().get_datagram_sender(self.id.into())
  }
}
