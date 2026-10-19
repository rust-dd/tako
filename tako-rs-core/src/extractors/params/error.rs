//! Error types for path parameter extraction and serde deserialization.

use std::fmt;

use http::StatusCode;
use serde::de::{self};

use crate::responder::Responder;

/// Why [`Params`](crate::extractors::params::Params) extraction failed.
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
  /// `MissingPathParams` answers 500 with a generic body that keeps routing details private.
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
    // A missing extension is a routing bug (500); a bad value is the caller's (400).
    let resp = ParamsError::MissingPathParams.into_response();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let resp = ParamsError::DeserializationError("bad".to_string()).into_response();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
  }
}
