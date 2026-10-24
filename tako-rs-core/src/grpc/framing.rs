use std::marker::PhantomData;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use bytes::Buf;
use bytes::Bytes;
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

/// Largest message a request frame may declare, the `grpc-go` and `tonic` default.
pub const MAX_GRPC_MESSAGE_SIZE: usize = 4 * 1024 * 1024;

/// Length of the `[compressed: u8][length: u32 BE]` prefix.
const PREFIX_LEN: usize = 5;

/// Rejection of a gRPC request, answered with a trailers-only response.
#[derive(Debug)]
pub enum GrpcError {
  /// Content-Type is not application/grpc.
  InvalidContentType,
  BodyReadError(String),
  /// The body ends inside a frame, or a frame is malformed.
  InvalidFrame,
  /// A frame declares more than [`MAX_GRPC_MESSAGE_SIZE`]; sent as `ResourceExhausted`, as
  /// `grpc-go` and `tonic` do, so clients may back off and retry.
  MessageTooLarge,
  DecodeError(String),
  /// A frame is compressed but no codec is configured; sent as `Unimplemented`.
  CompressionUnsupported,
}

impl GrpcError {
  fn status_parts(&self) -> (GrpcStatusCode, &'static str) {
    match self {
      GrpcError::InvalidContentType => (
        // `Unimplemented`, not `InvalidArgument`: the protocol is unsupported, not the payload.
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

/// Lets `?` turn a [`GrpcClientStream`](super::GrpcClientStream) error into a reply's status.
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

/// Encodes `msg` as one uncompressed length-prefixed frame; panics past `u32::MAX` bytes.
pub fn grpc_encode<T: Message>(msg: &T) -> Vec<u8> {
  let len = msg.encoded_len();
  let mut frame = Vec::with_capacity(PREFIX_LEN + len);
  frame.extend_from_slice(&frame_prefix(len));
  msg
    .encode(&mut frame)
    .expect("a Vec grows to fit any message");
  frame
}

/// Appends one frame to `buf`, so consecutive messages can share a DATA frame.
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

/// Decodes the first frame of `data`; compressed frames are rejected.
pub fn grpc_decode<T: Message + Default>(data: &[u8]) -> Result<T, GrpcError> {
  if data.len() < PREFIX_LEN {
    return Err(GrpcError::InvalidFrame);
  }
  if data[0] != 0 {
    return Err(GrpcError::CompressionUnsupported);
  }
  let msg_len = u32::from_be_bytes([data[1], data[2], data[3], data[4]]) as usize;

  if msg_len > MAX_GRPC_MESSAGE_SIZE {
    return Err(GrpcError::MessageTooLarge);
  }
  if data.len() < PREFIX_LEN + msg_len {
    return Err(GrpcError::InvalidFrame);
  }

  T::decode(&data[PREFIX_LEN..PREFIX_LEN + msg_len])
    .map_err(|e| GrpcError::DecodeError(e.to_string()))
}

/// Decodes frames as they arrive, sharing complete messages with their DATA chunk.
/// Fragmented messages buffer at most one message plus one chunk; the first error ends the stream.
pub(crate) struct FrameDecoder<T> {
  body: TakoBody,
  chunk: Bytes,
  buffer: BytesMut,
  done: bool,
  message: PhantomData<fn() -> T>,
}

impl<T> FrameDecoder<T> {
  pub(crate) fn new(body: TakoBody) -> Self {
    Self {
      body,
      chunk: Bytes::new(),
      buffer: BytesMut::new(),
      done: false,
      message: PhantomData,
    }
  }

  fn finish(&mut self, item: Option<Result<T, GrpcError>>) -> Poll<Option<Result<T, GrpcError>>> {
    self.done = true;
    self.chunk = Bytes::new();
    self.buffer = BytesMut::new();
    Poll::Ready(item)
  }

  #[inline]
  fn buffer_to(&mut self, len: usize) {
    let needed = len.saturating_sub(self.buffer.len()).min(self.chunk.len());
    self.buffer.extend_from_slice(&self.chunk[..needed]);
    self.chunk.advance(needed);
  }
}

impl<T: Message + Default> FrameDecoder<T> {
  fn buffered_message(&mut self) -> Option<Result<T, GrpcError>> {
    let mut frame = if self.buffer.is_empty() && self.chunk.len() >= PREFIX_LEN {
      let len = match frame_len(&self.chunk) {
        Ok(len) => len,
        Err(error) => return Some(Err(error)),
      };
      if self.chunk.len() < len {
        self.buffer.extend_from_slice(&self.chunk);
        self.chunk = Bytes::new();
        return None;
      }
      self.chunk.split_to(len)
    } else {
      self.buffer_to(PREFIX_LEN);
      if self.buffer.len() < PREFIX_LEN {
        return None;
      }
      let len = match frame_len(&self.buffer) {
        Ok(len) => len,
        Err(error) => return Some(Err(error)),
      };
      self.buffer_to(len);
      if self.buffer.len() < len {
        return None;
      }
      self.buffer.split().freeze()
    };
    frame.advance(PREFIX_LEN);
    Some(T::decode(frame).map_err(|e| GrpcError::DecodeError(e.to_string())))
  }
}

fn frame_len(prefix: &[u8]) -> Result<usize, GrpcError> {
  if prefix[0] != 0 {
    return Err(GrpcError::CompressionUnsupported);
  }
  let len = u32::from_be_bytes([prefix[1], prefix[2], prefix[3], prefix[4]]) as usize;
  if len > MAX_GRPC_MESSAGE_SIZE {
    return Err(GrpcError::MessageTooLarge);
  }
  Ok(PREFIX_LEN + len)
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
          if let Ok(data) = frame.into_data() {
            this.chunk = data;
          }
        }
        Poll::Ready(Some(Err(error))) => {
          return this.finish(Some(Err(GrpcError::BodyReadError(error.to_string()))));
        }
        Poll::Ready(None) => {
          let truncated = !this.buffer.is_empty() || !this.chunk.is_empty();
          return this.finish(truncated.then_some(Err(GrpcError::InvalidFrame)));
        }
        Poll::Pending => return Poll::Pending,
      }
    }
  }
}
