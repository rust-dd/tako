//! gRPC length-prefix framing: the message-size cap, encode/decode of a
//! single `[compressed][length][bytes]` frame, the incremental decoder that
//! reads messages off a request body, and the `GrpcError` type those
//! operations surface.

use std::marker::PhantomData;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use bytes::BytesMut;
use futures_util::Stream;
use http_body::Body;
use prost::Message;

use super::status::GrpcStatus;
use super::status::GrpcStatusCode;
use super::status::build_grpc_error_response;
use crate::body::TakoBody;
use crate::responder::Responder;
use crate::types::Response;

/// Cap on the `length` prefix of a single gRPC frame. Without it any client
/// can advertise a 4 GiB message and force the parser to either pre-allocate
/// that much space or treat the body as well-formed-but-truncated. 4 MiB
/// matches the default `grpc-go` and `tonic` server limits.
pub const MAX_GRPC_MESSAGE_SIZE: usize = 4 * 1024 * 1024;

/// Length of the `[compressed: u8][length: u32 BE]` prefix.
const PREFIX_LEN: usize = 5;

/// Error types for gRPC extraction.
#[derive(Debug)]
pub enum GrpcError {
  /// Content-Type is not application/grpc.
  InvalidContentType,
  /// Failed to read the request body.
  BodyReadError(String),
  /// gRPC frame is too short or malformed.
  InvalidFrame,
  /// Length-prefix advertises a message larger than [`MAX_GRPC_MESSAGE_SIZE`].
  ///
  /// Mapped to gRPC status `ResourceExhausted` (8) per the spec — `grpc-go`,
  /// `tonic`, and the upstream issue (grpc/grpc#23454) all use it for
  /// `received message larger than max`. Returning `InvalidArgument` would
  /// be wire-level wrong: clients that backoff-retry on `ResourceExhausted`
  /// would never retry on `InvalidArgument`.
  MessageTooLarge,
  /// Protobuf decoding failed.
  DecodeError(String),
  /// Frame's compressed flag was set but the server does not advertise
  /// any compression codec. Mapped to gRPC status `Unimplemented` per
  /// the spec (<https://grpc.io/docs/guides/wire>/) so clients fall back
  /// to uncompressed.
  CompressionUnsupported,
}

impl GrpcError {
  fn status_parts(&self) -> (GrpcStatusCode, &'static str) {
    match self {
      GrpcError::InvalidContentType => (
        // Spec maps wrong/missing content-type to `Unimplemented` (12) —
        // see PROTOCOL-HTTP2.md ("If Content-Type does not begin with
        // 'application/grpc', gRPC servers SHOULD respond with HTTP
        // status of 415 (Unsupported Media Type)"). grpcurl/Envoy
        // route on this distinction; `InvalidArgument` would suggest
        // a request-payload bug instead of an unsupported protocol.
        GrpcStatusCode::Unimplemented,
        "invalid content-type; expected application/grpc",
      ),
      GrpcError::BodyReadError(_) => (GrpcStatusCode::Internal, "failed to read request body"),
      GrpcError::InvalidFrame => (GrpcStatusCode::InvalidArgument, "malformed gRPC frame"),
      GrpcError::MessageTooLarge => (
        GrpcStatusCode::ResourceExhausted,
        "grpc message exceeds MAX_GRPC_MESSAGE_SIZE",
      ),
      GrpcError::DecodeError(_) => (
        GrpcStatusCode::InvalidArgument,
        "failed to decode protobuf message",
      ),
      GrpcError::CompressionUnsupported => (
        GrpcStatusCode::Unimplemented,
        "frame is compressed but no codec is configured",
      ),
    }
  }
}

/// Maps a request-side failure to the status a streaming handler sends, so
/// `?` works inside a reply stream that reads a [`GrpcClientStream`](super::GrpcClientStream).
impl From<GrpcError> for GrpcStatus {
  fn from(error: GrpcError) -> Self {
    let (code, message) = error.status_parts();
    GrpcStatus::error(code, message)
  }
}

impl Responder for GrpcError {
  fn into_response(self) -> Response {
    let (code, message) = self.status_parts();
    build_grpc_error_response(code, message)
  }
}

/// Encode a protobuf message with gRPC length-prefix framing.
///
/// Format: `[compressed: u8][length: u32 BE][message bytes]`
///
/// # Panics
///
/// Panics if the encoded message exceeds `u32::MAX` (≈ 4 GiB). gRPC's wire
/// format uses a 4-byte big-endian length prefix, so anything larger would
/// silently wrap to a wrong length and produce undecodable frames. The assert
/// turns that silent corruption into a loud server-side crash with a clear
/// site. (Outbound messages this large already indicate a serious
/// memory-pressure problem in the calling handler.)
pub fn grpc_encode<T: Message>(msg: &T) -> Vec<u8> {
  let len = msg.encoded_len();
  let mut frame = Vec::with_capacity(PREFIX_LEN + len);
  frame.extend_from_slice(&frame_prefix(len));
  msg
    .encode(&mut frame)
    .expect("a Vec grows to fit any message");
  frame
}

/// Appends one framed message to `buf`, which batches consecutive messages
/// into a single DATA frame.
pub(crate) fn encode_into<T: Message>(msg: &T, buf: &mut BytesMut) {
  let len = msg.encoded_len();
  buf.reserve(PREFIX_LEN + len);
  buf.extend_from_slice(&frame_prefix(len));
  msg
    .encode(buf)
    .expect("a BytesMut grows to fit any message");
}

fn frame_prefix(len: usize) -> [u8; PREFIX_LEN] {
  let Ok(len) = u32::try_from(len) else {
    panic!(
      "grpc_encode: message of {len} bytes exceeds u32::MAX (4 GiB) — gRPC length-prefix would wrap"
    );
  };
  let [a, b, c, d] = len.to_be_bytes();
  // The leading 0 is the compressed flag: messages are never compressed.
  [0, a, b, c, d]
}

/// Decode a gRPC length-prefix framed message.
///
/// Returns the decoded message and whether compression was indicated.
pub fn grpc_decode<T: Message + Default>(data: &[u8]) -> Result<(T, bool), GrpcError> {
  if data.len() < PREFIX_LEN {
    return Err(GrpcError::InvalidFrame);
  }

  let compressed = data[0] != 0;
  if compressed {
    return Err(GrpcError::CompressionUnsupported);
  }
  let msg_len = u32::from_be_bytes([data[1], data[2], data[3], data[4]]) as usize;

  if msg_len > MAX_GRPC_MESSAGE_SIZE {
    return Err(GrpcError::MessageTooLarge);
  }
  if data.len() < PREFIX_LEN + msg_len {
    return Err(GrpcError::InvalidFrame);
  }

  let msg = T::decode(&data[PREFIX_LEN..PREFIX_LEN + msg_len])
    .map_err(|e| GrpcError::DecodeError(e.to_string()))?;
  Ok((msg, compressed))
}

/// Reads length-prefixed messages off a request body as they arrive.
///
/// Buffers at most one message plus one body chunk. The first error ends the
/// stream, and a body that stops inside a frame yields
/// [`GrpcError::InvalidFrame`].
pub(crate) struct FrameDecoder<T> {
  body: TakoBody,
  buffer: BytesMut,
  done: bool,
  message: PhantomData<fn() -> T>,
}

impl<T> FrameDecoder<T> {
  pub(crate) fn new(body: TakoBody) -> Self {
    Self {
      body,
      buffer: BytesMut::new(),
      done: false,
      message: PhantomData,
    }
  }

  fn finish(&mut self, item: Option<Result<T, GrpcError>>) -> Poll<Option<Result<T, GrpcError>>> {
    self.done = true;
    self.buffer = BytesMut::new();
    Poll::Ready(item)
  }
}

impl<T: Message + Default> FrameDecoder<T> {
  fn buffered_message(&mut self) -> Option<Result<T, GrpcError>> {
    if self.buffer.len() < PREFIX_LEN {
      return None;
    }
    if self.buffer[0] != 0 {
      return Some(Err(GrpcError::CompressionUnsupported));
    }
    let len = u32::from_be_bytes([
      self.buffer[1],
      self.buffer[2],
      self.buffer[3],
      self.buffer[4],
    ]) as usize;
    if len > MAX_GRPC_MESSAGE_SIZE {
      return Some(Err(GrpcError::MessageTooLarge));
    }
    if self.buffer.len() < PREFIX_LEN + len {
      return None;
    }
    // Decoding from `Bytes` lets `bytes::Bytes` message fields share the
    // buffer instead of copying out of it.
    let frame = self.buffer.split_to(PREFIX_LEN + len).freeze();
    Some(T::decode(frame.slice(PREFIX_LEN..)).map_err(|e| GrpcError::DecodeError(e.to_string())))
  }
}

impl<T: Message + Default> Stream for FrameDecoder<T> {
  type Item = Result<T, GrpcError>;

  fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    let this = self.get_mut();
    loop {
      if this.done {
        return Poll::Ready(None);
      }
      match this.buffered_message() {
        Some(Ok(message)) => return Poll::Ready(Some(Ok(message))),
        Some(Err(error)) => return this.finish(Some(Err(error))),
        None => {}
      }
      match Pin::new(&mut this.body).poll_frame(cx) {
        Poll::Ready(Some(Ok(frame))) => {
          if let Some(data) = frame.data_ref() {
            this.buffer.extend_from_slice(data);
          }
        }
        Poll::Ready(Some(Err(error))) => {
          return this.finish(Some(Err(GrpcError::BodyReadError(error.to_string()))));
        }
        Poll::Ready(None) => {
          let truncated = !this.buffer.is_empty();
          return this.finish(truncated.then_some(Err(GrpcError::InvalidFrame)));
        }
        Poll::Pending => return Poll::Pending,
      }
    }
  }
}
