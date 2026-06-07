//! gRPC length-prefix framing: the message-size cap, encode/decode of a
//! single `[compressed][length][bytes]` frame, and the `GrpcError` type those
//! operations surface.

use prost::Message;

use super::status::GrpcStatusCode;
use super::status::build_grpc_error_response;
use crate::responder::Responder;
use crate::types::Response;

/// Cap on the `length` prefix of a single gRPC frame. Without it any client
/// can advertise a 4 GiB message and force the parser to either pre-allocate
/// that much space or treat the body as well-formed-but-truncated. 4 MiB
/// matches the default `grpc-go` and `tonic` server limits.
pub const MAX_GRPC_MESSAGE_SIZE: usize = 4 * 1024 * 1024;

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

impl Responder for GrpcError {
  fn into_response(self) -> Response {
    let (status_code, message) = match self {
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
    };

    build_grpc_error_response(status_code, message)
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
  let msg_bytes = msg.encode_to_vec();
  assert!(
    u32::try_from(msg_bytes.len()).is_ok(),
    "grpc_encode: message of {} bytes exceeds u32::MAX (4 GiB) — gRPC length-prefix would wrap",
    msg_bytes.len()
  );
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
  if compressed {
    return Err(GrpcError::CompressionUnsupported);
  }
  let msg_len = u32::from_be_bytes([data[1], data[2], data[3], data[4]]) as usize;

  if msg_len > MAX_GRPC_MESSAGE_SIZE {
    return Err(GrpcError::MessageTooLarge);
  }
  if data.len() < 5 + msg_len {
    return Err(GrpcError::InvalidFrame);
  }

  let msg = T::decode(&data[5..5 + msg_len]).map_err(|e| GrpcError::DecodeError(e.to_string()))?;
  Ok((msg, compressed))
}
