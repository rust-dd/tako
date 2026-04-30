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

use http::StatusCode;
use http_body_util::BodyExt;
use prost::Message;

use crate::body::TakoBody;
use crate::extractors::FromRequest;
use crate::responder::Responder;
use crate::types::{Request, Response};

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
      GrpcError::InvalidContentType => (GrpcStatusCode::InvalidArgument, "invalid content-type; expected application/grpc"),
      GrpcError::BodyReadError(_) => (GrpcStatusCode::Internal, "failed to read request body"),
      GrpcError::InvalidFrame => (GrpcStatusCode::InvalidArgument, "malformed gRPC frame"),
      GrpcError::DecodeError(_) => (GrpcStatusCode::InvalidArgument, "failed to decode protobuf message"),
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
      let msg_len = u32::from_be_bytes([
        body_bytes[1],
        body_bytes[2],
        body_bytes[3],
        body_bytes[4],
      ]) as usize;

      if body_bytes.len() < 5 + msg_len {
        return Err(GrpcError::InvalidFrame);
      }

      let message =
        T::decode(&body_bytes[5..5 + msg_len]).map_err(|e| GrpcError::DecodeError(e.to_string()))?;

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
      return build_grpc_error_response(
        self.status,
        self.error_message.as_deref().unwrap_or(""),
      );
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
  if !message.is_empty() {
    if let Ok(val) = http::HeaderValue::from_str(message) {
      resp.headers_mut().insert("grpc-message", val);
    }
  }
  resp
}
