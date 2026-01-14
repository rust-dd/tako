//! WebSocket connection handling for compio runtime.
//!
//! This module provides `TakoWsCompio` for handling WebSocket upgrade requests
//! when using the compio async runtime. It implements the WebSocket handshake
//! protocol according to RFC 6455 and integrates with compio-ws for message handling.

use std::future::Future;
use std::io::ErrorKind;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use compio_io::compat::SyncStream;
use compio_ws::tungstenite;
// Re-export Message for convenience
pub use compio_ws::tungstenite::Message;
use compio_ws::tungstenite::protocol::CloseFrame;
use compio_ws::tungstenite::protocol::Role;
use compio_ws::tungstenite::protocol::WebSocketConfig;
use futures_util::FutureExt;
use http::StatusCode;
use http::header;
use hyper::upgrade::Upgraded;
use sha1::Digest;
use sha1::Sha1;

use crate::body::TakoBody;
use crate::responder::Responder;
use crate::types::Request;
use crate::types::Response;

/// Wrapper to adapt hyper's Upgraded connection for compio-ws.
///
/// This struct wraps `hyper::upgrade::Upgraded` and implements the traits
/// needed by compio-ws to create a WebSocket stream.
pub struct UpgradedStream {
  inner: Upgraded,
}

impl UpgradedStream {
  /// Creates a new UpgradedStream from a hyper Upgraded connection.
  pub fn new(upgraded: Upgraded) -> Self {
    Self { inner: upgraded }
  }
}

impl compio_io::AsyncRead for UpgradedStream {
  async fn read<B: compio_buf::IoBufMut>(&mut self, mut buf: B) -> compio_buf::BufResult<usize, B> {
    use std::pin::Pin;
    use std::task::Context;

    use hyper::rt::Read;

    let slice = buf.as_mut_slice();
    let len = slice.len();

    // Create a safe buffer for reading
    let mut temp_buf = vec![0u8; len];

    let result = std::future::poll_fn(|cx: &mut Context<'_>| {
      let mut read_buf = hyper::rt::ReadBuf::new(&mut temp_buf);
      match Pin::new(&mut self.inner).poll_read(cx, read_buf.unfilled()) {
        std::task::Poll::Ready(Ok(())) => std::task::Poll::Ready(Ok(read_buf.filled().len())),
        std::task::Poll::Ready(Err(e)) => std::task::Poll::Ready(Err(e)),
        std::task::Poll::Pending => std::task::Poll::Pending,
      }
    })
    .await;

    match result {
      Ok(filled_len) => {
        // Copy from temp buffer to the actual buffer
        if filled_len > 0 {
          let dest =
            unsafe { std::slice::from_raw_parts_mut(slice.as_mut_ptr() as *mut u8, filled_len) };
          dest.copy_from_slice(&temp_buf[..filled_len]);
        }
        unsafe { buf.set_buf_init(filled_len) };
        (Ok(filled_len), buf).into()
      }
      Err(e) => (Err(e), buf).into(),
    }
  }
}

impl compio_io::AsyncWrite for UpgradedStream {
  async fn write<T: compio_buf::IoBuf>(&mut self, buf: T) -> compio_buf::BufResult<usize, T> {
    use std::pin::Pin;
    use std::task::Context;

    use hyper::rt::Write;

    let slice = buf.as_slice();

    let result =
      std::future::poll_fn(|cx: &mut Context<'_>| Pin::new(&mut self.inner).poll_write(cx, slice))
        .await;

    match result {
      Ok(n) => (Ok(n), buf).into(),
      Err(e) => (Err(e), buf).into(),
    }
  }

  async fn flush(&mut self) -> std::io::Result<()> {
    use std::pin::Pin;
    use std::task::Context;

    use hyper::rt::Write;

    std::future::poll_fn(|cx: &mut Context<'_>| Pin::new(&mut self.inner).poll_flush(cx)).await
  }

  async fn shutdown(&mut self) -> std::io::Result<()> {
    use std::pin::Pin;
    use std::task::Context;

    use hyper::rt::Write;

    std::future::poll_fn(|cx: &mut Context<'_>| Pin::new(&mut self.inner).poll_shutdown(cx)).await
  }
}

/// A WebSocket stream wrapper for compio that wraps tungstenite directly.
///
/// This type provides async WebSocket functionality by wrapping a tungstenite
/// WebSocket with a SyncStream adapter.
pub struct CompioWebSocket<S> {
  inner: tungstenite::WebSocket<SyncStream<S>>,
}

impl<S> CompioWebSocket<S>
where
  S: compio_io::AsyncRead + compio_io::AsyncWrite,
{
  /// Default buffer size (128 KiB).
  const DEFAULT_BUF_SIZE: usize = 128 * 1024;
  /// Default maximum buffer size (64 MiB).
  const DEFAULT_MAX_BUFFER: usize = 64 * 1024 * 1024;

  /// Creates a WebSocket stream from a raw socket without performing handshake.
  ///
  /// This is used after the HTTP upgrade handshake has already been completed.
  pub fn from_raw_socket(stream: S, role: Role, config: Option<WebSocketConfig>) -> Self {
    let sync_stream =
      SyncStream::with_limits(Self::DEFAULT_BUF_SIZE, Self::DEFAULT_MAX_BUFFER, stream);
    let ws = tungstenite::WebSocket::from_raw_socket(sync_stream, role, config);
    Self { inner: ws }
  }

  /// Sends a WebSocket message.
  pub async fn send(&mut self, message: Message) -> Result<(), tungstenite::Error> {
    // Send the message (buffers it)
    self.inner.send(message)?;
    // Flush the buffer to the network
    self.flush().await
  }

  /// Reads the next WebSocket message.
  pub async fn read(&mut self) -> Result<Message, tungstenite::Error> {
    loop {
      match self.inner.read() {
        Ok(msg) => {
          let _ = self.flush().await;
          return Ok(msg);
        }
        Err(tungstenite::Error::Io(ref e)) if e.kind() == ErrorKind::WouldBlock => {
          self
            .inner
            .get_mut()
            .fill_read_buf()
            .await
            .map_err(tungstenite::Error::Io)?;
        }
        Err(e) => {
          let _ = self.flush().await;
          return Err(e);
        }
      }
    }
  }

  /// Flushes pending messages.
  pub async fn flush(&mut self) -> Result<(), tungstenite::Error> {
    loop {
      match self.inner.flush() {
        Ok(()) => break,
        Err(tungstenite::Error::Io(ref e)) if e.kind() == ErrorKind::WouldBlock => {
          self
            .inner
            .get_mut()
            .flush_write_buf()
            .await
            .map_err(tungstenite::Error::Io)?;
        }
        Err(tungstenite::Error::ConnectionClosed) => break,
        Err(e) => return Err(e),
      }
    }
    self
      .inner
      .get_mut()
      .flush_write_buf()
      .await
      .map_err(tungstenite::Error::Io)?;
    Ok(())
  }

  /// Closes the WebSocket connection.
  pub async fn close(&mut self, close_frame: Option<CloseFrame>) -> Result<(), tungstenite::Error> {
    loop {
      match self.inner.close(close_frame.clone()) {
        Ok(()) => break,
        Err(tungstenite::Error::Io(ref e)) if e.kind() == ErrorKind::WouldBlock => {
          let sync_stream = self.inner.get_mut();
          let flushed = sync_stream
            .flush_write_buf()
            .await
            .map_err(tungstenite::Error::Io)?;
          if flushed == 0 {
            sync_stream
              .fill_read_buf()
              .await
              .map_err(tungstenite::Error::Io)?;
          }
        }
        Err(tungstenite::Error::ConnectionClosed) => break,
        Err(e) => return Err(e),
      }
    }
    self.flush().await
  }

  /// Returns a reference to the underlying stream.
  pub fn get_ref(&self) -> &S {
    self.inner.get_ref().get_ref()
  }

  /// Returns a mutable reference to the underlying stream.
  pub fn get_mut(&mut self) -> &mut S {
    self.inner.get_mut().get_mut()
  }
}

/// WebSocket connection handler for compio runtime.
///
/// `TakoWsCompio` manages the WebSocket handshake process and connection upgrade
/// when using the compio async runtime. It validates the WebSocket upgrade request,
/// performs the RFC 6455 handshake, and spawns a task to handle the WebSocket
/// connection using the provided handler function.
///
/// # Type Parameters
///
/// * `H` - Handler function type that processes the WebSocket connection
/// * `Fut` - Future type returned by the handler function
///
/// # Examples
///
/// ```rust,ignore
/// use tako::ws_compio::{TakoWsCompio, CompioWebSocket, UpgradedStream};
/// use tako::types::Request;
/// use tako::body::TakoBody;
/// use tako::responder::Responder;
/// use compio_ws::tungstenite::Message;
///
/// async fn echo_handler(mut ws: CompioWebSocket<UpgradedStream>) {
///     loop {
///         match ws.read().await {
///             Ok(Message::Text(text)) => {
///                 let _ = ws.send(Message::Text(format!("Echo: {}", text).into())).await;
///             }
///             Ok(Message::Close(_)) | Err(_) => break,
///             _ => {}
///         }
///     }
/// }
///
/// async fn handler(req: Request) -> impl Responder {
///     TakoWsCompio::new(req, echo_handler)
/// }
/// ```
#[doc(alias = "websocket")]
#[doc(alias = "ws")]
pub struct TakoWsCompio<H, Fut>
where
  H: FnOnce(CompioWebSocket<UpgradedStream>) -> Fut + 'static,
  Fut: Future<Output = ()> + 'static,
{
  request: Request,
  handler: H,
}

impl<H, Fut> TakoWsCompio<H, Fut>
where
  H: FnOnce(CompioWebSocket<UpgradedStream>) -> Fut + 'static,
  Fut: Future<Output = ()> + 'static,
{
  /// Creates a new WebSocket handler with the given request and handler function.
  pub fn new(request: Request, handler: H) -> Self {
    Self { request, handler }
  }
}

impl<H, Fut> Responder for TakoWsCompio<H, Fut>
where
  H: FnOnce(CompioWebSocket<UpgradedStream>) -> Fut + 'static,
  Fut: Future<Output = ()> + 'static,
{
  /// Converts the WebSocket handler into an HTTP response with upgrade protocol.
  fn into_response(self) -> Response {
    let (parts, body) = self.request.into_parts();
    let req = http::Request::from_parts(parts, body);

    let key = match req.headers().get("Sec-WebSocket-Key") {
      Some(k) => k,
      None => {
        return http::Response::builder()
          .status(StatusCode::BAD_REQUEST)
          .body(TakoBody::from("Missing Sec-WebSocket-Key".to_string()))
          .expect("valid bad request response");
      }
    };

    // RFC-6455 accept hash
    let accept = {
      let mut sha1 = Sha1::new();
      sha1.update(key.as_bytes());
      sha1.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
      STANDARD.encode(sha1.finalize())
    };

    let response = http::Response::builder()
      .status(StatusCode::SWITCHING_PROTOCOLS)
      .header(header::UPGRADE, "websocket")
      .header(header::CONNECTION, "Upgrade")
      .header("Sec-WebSocket-Accept", accept)
      .body(TakoBody::empty())
      .expect("valid WebSocket upgrade response");

    if let Some(on_upgrade) = req.extensions().get::<hyper::upgrade::OnUpgrade>().cloned() {
      let handler = self.handler;
      compio::runtime::spawn(async move {
        if let Ok(upgraded) = on_upgrade.await {
          let stream = UpgradedStream::new(upgraded);
          let ws = CompioWebSocket::from_raw_socket(stream, Role::Server, None);
          let _ = std::panic::AssertUnwindSafe(handler(ws))
            .catch_unwind()
            .await;
        }
      })
      .detach();
    }

    response
  }
}
