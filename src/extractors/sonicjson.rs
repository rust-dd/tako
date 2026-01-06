#![cfg_attr(docsrs, doc(cfg(feature = "sonic")))]

use http::StatusCode;

use crate::responder::Responder;
use crate::types::Response;

#[doc(alias = "sonicjson")]
pub struct SonicJson<T>(pub T);

/// Error type for SIMD JSON extraction.
#[derive(Debug)]
pub enum SonicJsonError {
  /// Request content type is not recognized as JSON.
  InvalidContentType,
  /// Content-Type header is missing from the request.
  MissingContentType,
  /// Failed to read the request body.
  BodyReadError(String),
  /// Failed to deserialize JSON using SIMD parser.
  DeserializationError(String),
}

impl Responder for SonicJsonError {
  /// Converts the error into an HTTP response.
  fn into_response(self) -> Response {
    match self {
      SonicJsonError::InvalidContentType => (
        StatusCode::BAD_REQUEST,
        "Invalid content type; expected JSON",
      )
        .into_response(),
      SonicJsonError::MissingContentType => {
        (StatusCode::BAD_REQUEST, "Missing content type header").into_response()
      }
      SonicJsonError::BodyReadError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Failed to read request body: {}", err),
      )
        .into_response(),
      SonicJsonError::DeserializationError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Failed to deserialize JSON: {}", err),
      )
        .into_response(),
    }
  }
}
