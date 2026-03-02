//! JSON request body extraction and deserialization for API endpoints.
//!
//! This module provides extractors for parsing JSON request bodies into strongly-typed Rust
//! structures using serde. It validates Content-Type headers, reads request bodies efficiently,
//! and provides detailed error information for malformed JSON or incorrect content types.
//! The extractor integrates seamlessly with serde's derive macros for automatic JSON
//! deserialization of complex data structures.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::json::Json;
//! use tako::extractors::FromRequest;
//! use tako::types::Request;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Debug, Deserialize, Serialize)]
//! struct CreateUser {
//!     name: String,
//!     email: String,
//!     age: u32,
//! }
//!
//! async fn create_user_handler(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
//!     let user_data: Json<CreateUser> = Json::from_request(&mut req).await?;
//!
//!     // Access the deserialized data
//!     println!("Creating user: {} ({})", user_data.0.name, user_data.0.email);
//!
//!     Ok(format!("User {} created successfully", user_data.0.name))
//! }
//!
//! // Nested JSON structures work seamlessly
//! #[derive(Deserialize)]
//! struct ApiRequest {
//!     action: String,
//!     payload: serde_json::Value,
//!     metadata: Option<std::collections::HashMap<String, String>>,
//! }
//! ```

use http::StatusCode;
use http::header::HeaderValue;
use http_body_util::BodyExt;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::body::TakoBody;
use crate::extractors::FromRequest;
use crate::responder::Responder;
use crate::types::Request;
use crate::types::Response;

/// Controls when the SIMD JSON parser is used for `Json<T>` extraction.
///
/// When the `simd` feature is enabled, `Json<T>` can automatically dispatch to
/// `sonic_rs` for large payloads. This enum lets you override that behavior
/// per-route via [`Route::simd_json`](crate::route::Route::simd_json).
///
/// If no config is set on a route, the default is `Threshold(2 MB)`.
///
/// # Examples
///
/// ```rust,ignore
/// use tako::extractors::json::SimdJsonMode;
///
/// // Always use SIMD on a heavy-payload route
/// router.route(Method::POST, "/api/ingest", ingest_handler)
///     .simd_json(SimdJsonMode::Always);
///
/// // Never use SIMD where payloads are tiny
/// router.route(Method::POST, "/api/ping", ping_handler)
///     .simd_json(SimdJsonMode::Never);
///
/// // Custom threshold (SIMD above 4 KB)
/// router.route(Method::POST, "/api/batch", batch_handler)
///     .simd_json(SimdJsonMode::Threshold(4096));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimdJsonMode {
  /// Always use the SIMD JSON parser, regardless of payload size.
  Always,
  /// Never use the SIMD JSON parser — always fall back to `serde_json`.
  Never,
  /// Use the SIMD JSON parser only when the payload exceeds this many bytes.
  ///
  /// This is the default behavior with a threshold of 2 MB (2,097,152 bytes).
  Threshold(usize),
}

impl Default for SimdJsonMode {
  fn default() -> Self {
    Self::Threshold(2 * 1024 * 1024) // 2 MB
  }
}

/// JSON request body extractor with automatic deserialization.
#[doc(alias = "json")]
pub struct Json<T>(pub T);

/// Error types for JSON extraction and deserialization.
///
/// This error type implements `std::error::Error` for integration with
/// error handling libraries and provides detailed error messages for
/// debugging JSON parsing issues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonError {
  /// Content-Type header is not application/json or compatible JSON type.
  InvalidContentType,
  /// Content-Type header is missing from the request.
  MissingContentType,
  /// Failed to read the request body (network error, timeout, etc.).
  BodyReadError(String),
  /// JSON deserialization failed (syntax error, type mismatch, etc.).
  DeserializationError(String),
}

impl std::fmt::Display for JsonError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::InvalidContentType => {
        write!(f, "invalid content type; expected application/json")
      }
      Self::MissingContentType => write!(f, "missing content type header"),
      Self::BodyReadError(err) => write!(f, "failed to read request body: {err}"),
      Self::DeserializationError(err) => write!(f, "failed to deserialize JSON: {err}"),
    }
  }
}

impl std::error::Error for JsonError {}

impl Responder for JsonError {
  /// Converts JSON extraction errors into appropriate HTTP error responses.
  fn into_response(self) -> crate::types::Response {
    match self {
      JsonError::InvalidContentType => (
        StatusCode::BAD_REQUEST,
        "Invalid content type; expected application/json",
      )
        .into_response(),
      JsonError::MissingContentType => {
        (StatusCode::BAD_REQUEST, "Missing content type header").into_response()
      }
      JsonError::BodyReadError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Failed to read request body: {err}"),
      )
        .into_response(),
      JsonError::DeserializationError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Failed to deserialize JSON: {err}"),
      )
        .into_response(),
    }
  }
}

use crate::extractors::is_json_content_type;

impl<'a, T> FromRequest<'a> for Json<T>
where
  T: DeserializeOwned + Send + 'static,
{
  type Error = JsonError;

  /// Extracts and deserializes JSON data from the HTTP request body.
  ///
  /// # Errors
  ///
  /// Returns [`JsonError`] if:
  /// - The Content-Type header is missing or not `application/json`.
  /// - The request body cannot be read.
  /// - The JSON cannot be deserialized into the target type.
  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      // Validate Content-Type header for JSON compatibility
      if !is_json_content_type(req.headers()) {
        return Err(JsonError::InvalidContentType);
      }

      // Read the complete request body into memory
      let body_bytes = req
        .body_mut()
        .collect()
        .await
        .map_err(|e| JsonError::BodyReadError(e.to_string()))?
        .to_bytes();

      // Deserialize JSON — use SIMD parser when the simd feature is enabled,
      // respecting per-route SimdJsonMode configuration from extensions.
      #[cfg(feature = "simd")]
      let data = {
        let mode = req
          .extensions()
          .get::<SimdJsonMode>()
          .copied()
          .unwrap_or_default();

        let use_simd = match mode {
          SimdJsonMode::Always => true,
          SimdJsonMode::Never => false,
          SimdJsonMode::Threshold(threshold) => body_bytes.len() >= threshold,
        };

        if use_simd {
          let mut owned = body_bytes.to_vec();
          sonic_rs::from_slice::<T>(&mut owned)
            .map_err(|e| JsonError::DeserializationError(e.to_string()))?
        } else {
          serde_json::from_slice(&body_bytes)
            .map_err(|e| JsonError::DeserializationError(e.to_string()))?
        }
      };
      #[cfg(not(feature = "simd"))]
      let data = serde_json::from_slice(&body_bytes)
        .map_err(|e| JsonError::DeserializationError(e.to_string()))?;

      Ok(Json(data))
    }
  }
}

impl<T> Responder for Json<T>
where
  T: Serialize,
{
  fn into_response(self) -> Response {
    match serde_json::to_vec(&self.0) {
      Ok(buf) => {
        let mut res = Response::new(TakoBody::from(buf));
        res.headers_mut().insert(
          http::header::CONTENT_TYPE,
          HeaderValue::from_static(mime::APPLICATION_JSON.as_ref()),
        );
        res
      }
      Err(err) => {
        let mut res = Response::new(crate::body::TakoBody::from(err.to_string()));
        *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        res.headers_mut().insert(
          http::header::CONTENT_TYPE,
          HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref()),
        );
        res
      }
    }
  }
}
