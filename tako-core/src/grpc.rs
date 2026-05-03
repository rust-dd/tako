#![cfg_attr(docsrs, doc(cfg(feature = "grpc")))]

//! gRPC support for unary RPCs over HTTP/2.
//!
//! Provides `GrpcRequest<T>` extractor and `GrpcResponse<T>` responder that
//! handle gRPC framing (length-prefixed protobuf messages) and integrate with
//! Tako's handler system.
//!
//! # Examples
//!
//! ```rust,ignore
//! use tako::grpc::{GrpcRequest, GrpcResponse};
//! use prost::Message;
//!
//! #[derive(Clone, PartialEq, Message)]
//! struct HelloRequest {
//!     #[prost(string, tag = "1")]
//!     pub name: String,
//! }
//!
//! #[derive(Clone, PartialEq, Message)]
//! struct HelloReply {
//!     #[prost(string, tag = "1")]
//!     pub message: String,
//! }
//!
//! async fn say_hello(req: GrpcRequest<HelloRequest>) -> GrpcResponse<HelloReply> {
//!     GrpcResponse::ok(HelloReply {
//!         message: format!("Hello, {}!", req.message.name),
//!     })
//! }
//!
//! // Register on router:
//! // router.route(Method::POST, "/helloworld.Greeter/SayHello", say_hello);
//! ```

/// `grpc.health.v1` scaffolding.
pub mod health;
/// gRPC-specific interceptor pattern.
pub mod interceptor;
/// `grpc.reflection.v1` scaffolding.
pub mod reflection;
/// gRPC-Web bridge translating browser-friendly framing to canonical gRPC.
pub mod web;

use std::convert::Infallible;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;
use std::time::Instant;

use bytes::Bytes;
use bytes::BytesMut;
use futures_util::Stream;
use futures_util::StreamExt;
use http::HeaderMap;
use http::StatusCode;
use http_body::Frame;
use http_body_util::BodyExt;
use http_body_util::StreamBody;
use prost::Message;

use crate::body::TakoBody;
use crate::extractors::FromRequest;
use crate::responder::Responder;
use crate::types::Request;
use crate::types::Response;

/// gRPC status codes.
///
/// See <https://grpc.github.io/grpc/core/md_doc_statuscodes.html>
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GrpcStatusCode {
  Ok = 0,
  Cancelled = 1,
  Unknown = 2,
  InvalidArgument = 3,
  DeadlineExceeded = 4,
  NotFound = 5,
  AlreadyExists = 6,
  PermissionDenied = 7,
  ResourceExhausted = 8,
  FailedPrecondition = 9,
  Aborted = 10,
  OutOfRange = 11,
  Unimplemented = 12,
  Internal = 13,
  Unavailable = 14,
  DataLoss = 15,
  Unauthenticated = 16,
}

/// gRPC request extractor.
///
/// Extracts and decodes a gRPC-framed protobuf message from the request body.
/// Validates that the content-type is `application/grpc`.
pub struct GrpcRequest<T: Message + Default> {
  /// The decoded protobuf message.
  pub message: T,
}

/// Error types for gRPC extraction.
#[derive(Debug)]
pub enum GrpcError {
  /// Content-Type is not application/grpc.
  InvalidContentType,
  /// Failed to read the request body.
  BodyReadError(String),
  /// gRPC frame is too short or malformed.
  InvalidFrame,
  /// Protobuf decoding failed.
  DecodeError(String),
}

impl Responder for GrpcError {
  fn into_response(self) -> Response {
    let (status_code, message) = match self {
      GrpcError::InvalidContentType => (
        GrpcStatusCode::InvalidArgument,
        "invalid content-type; expected application/grpc",
      ),
      GrpcError::BodyReadError(_) => (GrpcStatusCode::Internal, "failed to read request body"),
      GrpcError::InvalidFrame => (GrpcStatusCode::InvalidArgument, "malformed gRPC frame"),
      GrpcError::DecodeError(_) => (
        GrpcStatusCode::InvalidArgument,
        "failed to decode protobuf message",
      ),
    };

    build_grpc_error_response(status_code, message)
  }
}

impl<'a, T> FromRequest<'a> for GrpcRequest<T>
where
  T: Message + Default + Send + 'static,
{
  type Error = GrpcError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      // Validate content-type
      let ct = req
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

      if !ct.starts_with("application/grpc") {
        return Err(GrpcError::InvalidContentType);
      }

      // Read body
      let body_bytes = req
        .body_mut()
        .collect()
        .await
        .map_err(|e| GrpcError::BodyReadError(e.to_string()))?
        .to_bytes();

      // Decode gRPC frame: 1 byte compressed + 4 bytes length + message
      if body_bytes.len() < 5 {
        return Err(GrpcError::InvalidFrame);
      }

      let _compressed = body_bytes[0];
      let msg_len =
        u32::from_be_bytes([body_bytes[1], body_bytes[2], body_bytes[3], body_bytes[4]]) as usize;

      if body_bytes.len() < 5 + msg_len {
        return Err(GrpcError::InvalidFrame);
      }

      let message = T::decode(&body_bytes[5..5 + msg_len])
        .map_err(|e| GrpcError::DecodeError(e.to_string()))?;

      Ok(GrpcRequest { message })
    }
  }
}

/// gRPC response wrapper.
///
/// Encodes a protobuf message with gRPC framing and sets appropriate headers.
pub struct GrpcResponse<T: Message> {
  /// The response message (None for error-only responses).
  message: Option<T>,
  /// gRPC status code.
  status: GrpcStatusCode,
  /// Optional error message.
  error_message: Option<String>,
}

impl<T: Message> GrpcResponse<T> {
  /// Creates a successful gRPC response with the given message.
  pub fn ok(message: T) -> Self {
    Self {
      message: Some(message),
      status: GrpcStatusCode::Ok,
      error_message: None,
    }
  }

  /// Creates an error gRPC response with the given status and message.
  pub fn error(status: GrpcStatusCode, message: impl Into<String>) -> Self {
    Self {
      message: None,
      status,
      error_message: Some(message.into()),
    }
  }
}

impl<T: Message> Responder for GrpcResponse<T> {
  fn into_response(self) -> Response {
    if self.status != GrpcStatusCode::Ok {
      return build_grpc_error_response(self.status, self.error_message.as_deref().unwrap_or(""));
    }

    let body_bytes = match self.message {
      Some(msg) => grpc_encode(&msg),
      None => Vec::new(),
    };

    let mut resp = Response::new(TakoBody::from(body_bytes));
    *resp.status_mut() = StatusCode::OK;
    resp.headers_mut().insert(
      http::header::CONTENT_TYPE,
      http::HeaderValue::from_static("application/grpc"),
    );
    // gRPC uses trailers for status. Since we're using HTTP/1.1-compatible
    // responses, we put the status in headers as a fallback.
    if let Ok(val) = http::HeaderValue::from_str(&(self.status as u8).to_string()) {
      resp.headers_mut().insert("grpc-status", val);
    }
    resp
  }
}

/// Encode a protobuf message with gRPC length-prefix framing.
///
/// Format: `[compressed: u8][length: u32 BE][message bytes]`
pub fn grpc_encode<T: Message>(msg: &T) -> Vec<u8> {
  let msg_bytes = msg.encode_to_vec();
  let len = msg_bytes.len() as u32;

  let mut frame = Vec::with_capacity(5 + msg_bytes.len());
  frame.push(0); // not compressed
  frame.extend_from_slice(&len.to_be_bytes());
  frame.extend_from_slice(&msg_bytes);
  frame
}

/// Decode a gRPC length-prefix framed message.
///
/// Returns the decoded message and whether compression was indicated.
pub fn grpc_decode<T: Message + Default>(data: &[u8]) -> Result<(T, bool), GrpcError> {
  if data.len() < 5 {
    return Err(GrpcError::InvalidFrame);
  }

  let compressed = data[0] != 0;
  let msg_len = u32::from_be_bytes([data[1], data[2], data[3], data[4]]) as usize;

  if data.len() < 5 + msg_len {
    return Err(GrpcError::InvalidFrame);
  }

  let msg = T::decode(&data[5..5 + msg_len]).map_err(|e| GrpcError::DecodeError(e.to_string()))?;
  Ok((msg, compressed))
}

fn build_grpc_error_response(status: GrpcStatusCode, message: &str) -> Response {
  let mut resp = Response::new(TakoBody::empty());
  *resp.status_mut() = StatusCode::OK; // gRPC always uses 200 OK at HTTP level
  resp.headers_mut().insert(
    http::header::CONTENT_TYPE,
    http::HeaderValue::from_static("application/grpc"),
  );
  if let Ok(val) = http::HeaderValue::from_str(&(status as u8).to_string()) {
    resp.headers_mut().insert("grpc-status", val);
  }
  if !message.is_empty()
    && let Ok(val) = http::HeaderValue::from_str(message)
  {
    resp.headers_mut().insert("grpc-message", val);
  }
  resp
}

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

/// gRPC status payload (status code + optional message) used in trailers.
#[derive(Debug, Clone)]
pub struct GrpcStatus {
  pub code: GrpcStatusCode,
  pub message: Option<String>,
}

impl GrpcStatus {
  pub fn ok() -> Self {
    Self {
      code: GrpcStatusCode::Ok,
      message: None,
    }
  }

  pub fn error(code: GrpcStatusCode, message: impl Into<String>) -> Self {
    Self {
      code,
      message: Some(message.into()),
    }
  }

  fn write_trailers(&self) -> HeaderMap {
    let mut t = HeaderMap::new();
    if let Ok(v) = http::HeaderValue::from_str(&(self.code as u8).to_string()) {
      t.insert("grpc-status", v);
    }
    if let Some(msg) = self.message.as_deref()
      && let Ok(v) = http::HeaderValue::from_str(msg)
    {
      t.insert("grpc-message", v);
    }
    t
  }
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
    let stream = self.stream.map(|item| match item {
      Ok(msg) => {
        let bytes = grpc_encode(&msg);
        Ok::<_, Infallible>(Frame::data(Bytes::from(bytes)))
      }
      Err(status) => Ok(Frame::trailers(status.write_trailers())),
    });

    // After the user stream exhausts, append a final `grpc-status: 0` trailer.
    let trailer = futures_util::stream::once(async {
      Ok::<_, Infallible>(Frame::trailers(GrpcStatus::ok().write_trailers()))
    });
    let combined = stream.chain(trailer);

    let mut resp = http::Response::builder()
      .status(StatusCode::OK)
      .header(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/grpc"),
      )
      .body(TakoBody::new(StreamBody::new(combined)))
      .expect("valid grpc streaming response");
    let headers = resp.headers_mut();
    for (k, v) in self.initial_metadata.iter() {
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
        if this.buffer.len() >= 5 + msg_len {
          let _compressed = this.buffer[0];
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

/// gRPC deadline propagated from the `grpc-timeout` request header.
#[derive(Debug, Clone, Copy)]
pub struct GrpcDeadline(pub Instant);

/// Parse the `grpc-timeout` header value (e.g. `"100m"`, `"5S"`, `"1H"`).
pub fn parse_grpc_timeout(value: &str) -> Option<Duration> {
  let value = value.trim();
  if value.is_empty() {
    return None;
  }
  let (num, unit) = value.split_at(value.len() - 1);
  let num: u64 = num.parse().ok()?;
  let dur = match unit {
    "n" => Duration::from_nanos(num),
    "u" => Duration::from_micros(num),
    "m" => Duration::from_millis(num),
    "S" => Duration::from_secs(num),
    "M" => Duration::from_secs(num * 60),
    "H" => Duration::from_secs(num * 3600),
    _ => return None,
  };
  Some(dur)
}

/// Extract the deadline (if any) from a request's `grpc-timeout` header.
///
/// Inserts a [`GrpcDeadline`] into request extensions when present so handlers
/// and middleware can honor the cancellation contract.
pub fn read_grpc_deadline(req: &mut Request) -> Option<GrpcDeadline> {
  let raw = req
    .headers()
    .get("grpc-timeout")
    .and_then(|v| v.to_str().ok())?;
  let dur = parse_grpc_timeout(raw)?;
  let deadline = GrpcDeadline(Instant::now() + dur);
  req.extensions_mut().insert(deadline);
  Some(deadline)
}
