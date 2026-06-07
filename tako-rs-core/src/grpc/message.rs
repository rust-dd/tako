//! Unary gRPC request extractor (`GrpcRequest`) and response responder
//! (`GrpcResponse`) over a single length-prefixed protobuf frame.

use http::StatusCode;
use http_body_util::BodyExt;
use prost::Message;

use super::GrpcError;
use super::framing::MAX_GRPC_MESSAGE_SIZE;
use super::framing::grpc_encode;
use super::status::GrpcStatusCode;
use super::status::build_grpc_error_response;
use crate::body::TakoBody;
use crate::extractors::FromRequest;
use crate::responder::Responder;
use crate::types::Request;
use crate::types::Response;

/// gRPC request extractor.
///
/// Extracts and decodes a gRPC-framed protobuf message from the request body.
/// Validates that the content-type is `application/grpc`.
pub struct GrpcRequest<T: Message + Default> {
  /// The decoded protobuf message.
  pub message: T,
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

      if body_bytes[0] != 0 {
        return Err(GrpcError::CompressionUnsupported);
      }
      let msg_len =
        u32::from_be_bytes([body_bytes[1], body_bytes[2], body_bytes[3], body_bytes[4]]) as usize;

      if msg_len > MAX_GRPC_MESSAGE_SIZE {
        return Err(GrpcError::MessageTooLarge);
      }
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
