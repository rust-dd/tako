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
use crate::extractors::Entries;
use crate::extractors::FromRequest;
use crate::extractors::body::BodyLimit;
use crate::responder::Responder;
use crate::types::Request;
use crate::types::Response;

/// Exactly one protobuf message, with no trailing data, under the request's [`BodyLimit`].
/// Accepts `application/grpc` and `application/grpc+proto`, with MIME parameters.
pub struct GrpcRequest<T: Message + Default> {
  pub message: T,
}

impl<'a, T> FromRequest<'a> for GrpcRequest<T>
where
  T: Message + Default + Send + 'static,
{
  type Error = GrpcError;
  const ENTRIES: Entries = Entries::BODY_LIMIT;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      if !is_grpc_content_type(req) {
        return Err(GrpcError::InvalidContentType);
      }
      let limit = req
        .extensions()
        .get::<BodyLimit>()
        .copied()
        .unwrap_or_default();
      let body = std::mem::take(req.body_mut());
      let body = match limit.0 {
        Some(limit) => TakoBody::new(http_body_util::Limited::new(body, limit)),
        None => body,
      };
      let mut messages = FrameDecoder::<T>::new(body);
      let message = messages.next().await.ok_or(GrpcError::InvalidFrame)??;
      match messages.next().await {
        None => Ok(GrpcRequest { message }),
        Some(Ok(_)) => Err(GrpcError::InvalidFrame),
        Some(Err(error)) => Err(error),
      }
    }
  }
}

pub(crate) fn is_grpc_content_type(req: &Request) -> bool {
  req
    .headers()
    .get(http::header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .and_then(|ct| ct.parse::<mime::Mime>().ok())
    .is_some_and(|ct| {
      ct.type_() == mime::APPLICATION
        && ct.subtype() == "grpc"
        && ct.suffix().is_none_or(|suffix| suffix == "proto")
    })
}

/// Unary reply: one message with `grpc-status` in trailers, or a trailers-only error.
pub struct GrpcResponse<T: Message> {
  message: Option<T>,
  status: GrpcStatusCode,
  error_message: Option<String>,
}

impl<T: Message> GrpcResponse<T> {
  pub fn ok(message: T) -> Self {
    Self {
      message: Some(message),
      status: GrpcStatusCode::Ok,
      error_message: None,
    }
  }

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
