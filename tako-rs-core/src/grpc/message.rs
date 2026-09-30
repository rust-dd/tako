//! Unary gRPC request extractor (`GrpcRequest`) and response responder
//! (`GrpcResponse`) over a single length-prefixed protobuf frame.

use std::convert::Infallible;

use bytes::Bytes;
use futures_util::StreamExt;
use http_body::Frame;
use prost::Message;

use super::GrpcError;
use super::framing::FrameDecoder;
use super::framing::grpc_encode;
use super::status::GrpcStatus;
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
      if !is_grpc_content_type(req) {
        return Err(GrpcError::InvalidContentType);
      }
      // Reads only as far as the first message, so the body is bounded by
      // `MAX_GRPC_MESSAGE_SIZE` rather than buffered whole.
      let mut messages = FrameDecoder::<T>::new(std::mem::take(req.body_mut()));
      match messages.next().await {
        Some(Ok(message)) => Ok(GrpcRequest { message }),
        Some(Err(error)) => Err(error),
        None => Err(GrpcError::InvalidFrame),
      }
    }
  }
}

pub(crate) fn is_grpc_content_type(req: &Request) -> bool {
  req
    .headers()
    .get(http::header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .is_some_and(|ct| ct.starts_with("application/grpc"))
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
    let Some(message) = self.message.filter(|_| self.status == GrpcStatusCode::Ok) else {
      return build_grpc_error_response(self.status, self.error_message.as_deref().unwrap_or(""));
    };

    // `grpc-status` travels in trailers after the message; clients such as
    // grpc-go reject a stream that ends on DATA without them.
    let frames = [
      Frame::data(Bytes::from(grpc_encode(&message))),
      Frame::trailers(GrpcStatus::ok().write_trailers()),
    ];
    let body =
      TakoBody::from_try_stream(futures_util::stream::iter(frames.map(Ok::<_, Infallible>)));
    let mut resp = Response::new(body);
    resp.headers_mut().insert(
      http::header::CONTENT_TYPE,
      http::HeaderValue::from_static("application/grpc"),
    );
    resp
  }
}
