//! Streaming gRPC integration: server-streaming responder, client-streaming
//! extractor, the internal frame de-framer, and the bidirectional scaffold
//! that wires a handler's inbound and outbound streams together.

use std::convert::Infallible;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use bytes::Bytes;
use bytes::BytesMut;
use futures_util::Stream;
use futures_util::StreamExt;
use http::HeaderMap;
use http::StatusCode;
use http_body::Frame;
use http_body_util::StreamBody;
use prost::Message;

use super::GrpcError;
use super::framing::MAX_GRPC_MESSAGE_SIZE;
use super::framing::grpc_encode;
use super::status::GrpcStatus;
use crate::body::TakoBody;
use crate::extractors::FromRequest;
use crate::responder::Responder;
use crate::types::Request;
use crate::types::Response;

/// Server-streaming gRPC response.
///
/// Encodes each `Ok` item with the standard length-prefix framing and emits a
/// final HTTP/2 trailer carrying `grpc-status` (`Ok` if the stream terminates
/// cleanly) and `grpc-message` when applicable.
pub struct GrpcServerStream<S, T>
where
  S: Stream<Item = Result<T, GrpcStatus>> + Send + 'static,
  T: Message + Send + 'static,
{
  pub stream: S,
  /// Server metadata sent as response headers (initial metadata).
  pub initial_metadata: HeaderMap,
}

impl<S, T> GrpcServerStream<S, T>
where
  S: Stream<Item = Result<T, GrpcStatus>> + Send + 'static,
  T: Message + Send + 'static,
{
  pub fn new(stream: S) -> Self {
    Self {
      stream,
      initial_metadata: HeaderMap::new(),
    }
  }

  pub fn with_metadata(mut self, headers: HeaderMap) -> Self {
    self.initial_metadata = headers;
    self
  }
}

impl<S, T> Responder for GrpcServerStream<S, T>
where
  S: Stream<Item = Result<T, GrpcStatus>> + Send + 'static,
  T: Message + Send + 'static,
{
  fn into_response(self) -> Response {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;

    // Track whether the user stream already emitted a terminal `grpc-status`
    // trailer (i.e. ended in `Err(status)`). Without this, the unconditional
    // OK trailer below would double the trailer headers — RFC §8.1 / the
    // gRPC HTTP/2 mapping disallows two `grpc-status` values per response.
    let error_emitted = Arc::new(AtomicBool::new(false));
    let mark_err = error_emitted.clone();
    let stream = self.stream.map(move |item| match item {
      Ok(msg) => {
        let bytes = grpc_encode(&msg);
        Ok::<_, Infallible>(Frame::data(Bytes::from(bytes)))
      }
      Err(status) => {
        mark_err.store(true, Ordering::Release);
        Ok(Frame::trailers(status.write_trailers()))
      }
    });

    // After the user stream exhausts, append a final `grpc-status: 0`
    // trailer — but only if no error trailer was emitted upstream.
    let check_err = error_emitted.clone();
    let mut once = false;
    let trailer = futures_util::stream::iter(std::iter::from_fn(move || {
      if once {
        None
      } else {
        once = true;
        if check_err.load(Ordering::Acquire) {
          None
        } else {
          Some(Ok::<_, Infallible>(Frame::trailers(
            GrpcStatus::ok().write_trailers(),
          )))
        }
      }
    }));
    let combined = stream.chain(trailer);

    // SAFETY of `.expect(...)`: `Response::builder().status(...).header(...).body(...)`
    // can only return Err if a `header_name`/`header_value` fails to convert.
    // Here both inputs are `HeaderName::from_static`/`HeaderValue::from_static`,
    // which are pre-validated at compile time. The status code and body are
    // infallible.
    //
    // If you ADD a `.header(dynamic_name, dynamic_value)` to this builder
    // chain, the panic message becomes misleading — the failure mode is no
    // longer impossible. In that case, switch to `.body(...)?` + propagate
    // via `Result<Response, _>` (callers can map back via Responder), or
    // construct `http::Response::new(...)` directly + setters.
    let mut resp = http::Response::builder()
      .status(StatusCode::OK)
      .header(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/grpc"),
      )
      .body(TakoBody::new(StreamBody::new(combined)))
      .expect("static headers + body construction is infallible");
    let headers = resp.headers_mut();
    for (k, v) in &self.initial_metadata {
      headers.insert(k.clone(), v.clone());
    }
    resp
  }
}

/// Client-streaming gRPC extractor.
///
/// Wraps the request body into a `Stream<Item = Result<T, GrpcError>>` so a
/// handler can iterate over framed protobuf messages.
pub struct GrpcClientStream<T: Message + Default + Send + 'static> {
  pub stream: Pin<Box<dyn Stream<Item = Result<T, GrpcError>> + Send>>,
}

impl<'a, T> FromRequest<'a> for GrpcClientStream<T>
where
  T: Message + Default + Send + 'static,
{
  type Error = GrpcError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      let ct = req
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
      if !ct.starts_with("application/grpc") {
        return Err(GrpcError::InvalidContentType);
      }

      // Take the body out of the request — `into_body` is not directly
      // available without owning the request; we drain incrementally instead.
      // Collect a one-shot producer and parse multiple frames out of it.
      let body = std::mem::take(req.body_mut());
      let stream = GrpcFrameStream::new(body);
      Ok(GrpcClientStream {
        stream: Box::pin(stream),
      })
    }
  }
}

struct GrpcFrameStream<T> {
  body: TakoBody,
  buffer: BytesMut,
  finished: bool,
  _marker: std::marker::PhantomData<fn() -> T>,
}

impl<T> GrpcFrameStream<T> {
  fn new(body: TakoBody) -> Self {
    Self {
      body,
      buffer: BytesMut::new(),
      finished: false,
      _marker: std::marker::PhantomData,
    }
  }
}

impl<T> Stream for GrpcFrameStream<T>
where
  T: Message + Default,
{
  type Item = Result<T, GrpcError>;

  fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    let this = self.get_mut();
    loop {
      // Try to emit a frame from the buffer.
      if this.buffer.len() >= 5 {
        let msg_len = u32::from_be_bytes([
          this.buffer[1],
          this.buffer[2],
          this.buffer[3],
          this.buffer[4],
        ]) as usize;
        if msg_len > MAX_GRPC_MESSAGE_SIZE {
          return Poll::Ready(Some(Err(GrpcError::MessageTooLarge)));
        }
        if this.buffer.len() >= 5 + msg_len {
          if this.buffer[0] != 0 {
            return Poll::Ready(Some(Err(GrpcError::CompressionUnsupported)));
          }
          let payload = this.buffer.split_to(5 + msg_len);
          let msg_bytes = &payload[5..5 + msg_len];
          return match T::decode(msg_bytes) {
            Ok(m) => Poll::Ready(Some(Ok(m))),
            Err(e) => Poll::Ready(Some(Err(GrpcError::DecodeError(e.to_string())))),
          };
        }
      }

      if this.finished {
        return Poll::Ready(None);
      }

      // Pull more bytes off the body.
      let mut body = Pin::new(&mut this.body);
      match http_body::Body::poll_frame(body.as_mut(), cx) {
        Poll::Ready(Some(Ok(frame))) => {
          if let Some(data) = frame.data_ref() {
            this.buffer.extend_from_slice(data);
          }
        }
        Poll::Ready(Some(Err(e))) => {
          return Poll::Ready(Some(Err(GrpcError::BodyReadError(e.to_string()))));
        }
        Poll::Ready(None) => {
          this.finished = true;
        }
        Poll::Pending => return Poll::Pending,
      }
    }
  }
}

/// Bidirectional gRPC handler scaffold.
///
/// Combines a [`GrpcClientStream`] (for inbound) with a `GrpcServerStream`
/// builder (for outbound). The handler reads inbound frames as needed and
/// drives the outbound stream as a Responder.
pub struct GrpcBidi<Req, Resp>
where
  Req: Message + Default + Send + 'static,
  Resp: Message + Send + 'static,
{
  pub inbound: GrpcClientStream<Req>,
  pub _phantom: std::marker::PhantomData<Resp>,
}

impl<'a, Req, Resp> FromRequest<'a> for GrpcBidi<Req, Resp>
where
  Req: Message + Default + Send + 'static,
  Resp: Message + Send + 'static,
{
  type Error = GrpcError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      Ok(GrpcBidi {
        inbound: GrpcClientStream::<Req>::from_request(req).await?,
        _phantom: std::marker::PhantomData,
      })
    }
  }
}
