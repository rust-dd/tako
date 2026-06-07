//! Error types for path parameter extraction and serde deserialization.

use std::fmt;

use http::StatusCode;
use serde::de::{self};

use crate::responder::Responder;

/// Error types for path parameter extraction and deserialization.
///
/// This error type implements `std::error::Error` for integration with
/// error handling libraries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamsError {
  /// Path parameters not found in request extensions (internal routing error).
  MissingPathParams,
  /// Parameter deserialization failed (type mismatch, missing field, etc.).
  DeserializationError(String),
}

impl std::fmt::Display for ParamsError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::MissingPathParams => write!(f, "path parameters not found in request extensions"),
      Self::DeserializationError(err) => {
        write!(f, "failed to deserialize path parameters: {err}")
      }
    }
  }
}

impl std::error::Error for ParamsError {}

impl Responder for ParamsError {
  /// Converts path parameter errors into appropriate HTTP error responses.
  ///
  /// `MissingPathParams` is an internal-routing condition (the router did not
  /// populate the extension), so the response body is intentionally generic
  /// — the detailed framing ("request extensions") stays in the
  /// `Display`/`tracing` form so it does not leak through the wire.
  fn into_response(self) -> crate::types::Response {
    match self {
      ParamsError::MissingPathParams => {
        (StatusCode::INTERNAL_SERVER_ERROR, "Internal routing error").into_response()
      }
      ParamsError::DeserializationError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Failed to deserialize path parameters: {err}"),
      )
        .into_response(),
    }
  }
}

// Custom error type for the deserializer
#[derive(Debug)]
pub(crate) struct PathParamsDeError(String);

impl fmt::Display for PathParamsDeError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(&self.0)
  }
}

impl std::error::Error for PathParamsDeError {}

impl de::Error for PathParamsDeError {
  fn custom<T: fmt::Display>(msg: T) -> Self {
    PathParamsDeError(msg.to_string())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn params_error_responder_status_codes() {
    // Missing PathParams in request extensions is a routing-internal bug,
    // so the responder maps it to 500 rather than 400. Deserialization
    // failure is caller-visible and stays at 400.
    let resp = ParamsError::MissingPathParams.into_response();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let resp = ParamsError::DeserializationError("bad".to_string()).into_response();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
  }
}
