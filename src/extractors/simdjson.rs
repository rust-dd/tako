//! SIMD-accelerated JSON extraction from HTTP request bodies.
//!
//! This module provides the [`SimdJson`] extractor that leverages SIMD-accelerated JSON
//! parsing via the `simd_json` crate for high-performance deserialization of request bodies.
//! It offers similar functionality to standard JSON extractors but with potentially
//! better performance for large JSON payloads.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::simdjson::SimdJson;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Deserialize, Serialize)]
//! struct User {
//!     name: String,
//!     email: String,
//!     age: u32,
//! }
//!
//! async fn create_user_handler(SimdJson(user): SimdJson<User>) -> SimdJson<User> {
//!     println!("Creating user: {}", user.name);
//!     // Process user creation...
//!     SimdJson(user)
//! }
//! ```

use http::{
  HeaderMap, StatusCode,
  header::{self, HeaderValue},
};
use http_body_util::BodyExt;
use serde::{Serialize, de::DeserializeOwned};

use crate::{
  body::TakoBody,
  extractors::FromRequest,
  responder::Responder,
  types::{Request, Response},
};

/// An extractor that (de)serializes JSON using SIMD-accelerated parsing.
///
/// `SimdJson<T>` behaves similarly to standard JSON extractors but leverages
/// SIMD-accelerated parsing for potentially higher performance, especially with
/// large JSON payloads. It automatically handles content-type validation,
/// request body reading, and deserialization.
///
/// The extractor also implements [`Responder`], allowing it to be returned
/// directly from handler functions for JSON responses.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::simdjson::SimdJson;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Deserialize, Serialize)]
/// struct ApiResponse {
///     success: bool,
///     message: String,
/// }
///
/// async fn api_handler(SimdJson(request): SimdJson<ApiResponse>) -> SimdJson<ApiResponse> {
///     // Process the request...
///     SimdJson(ApiResponse {
///         success: true,
///         message: "Request processed successfully".to_string(),
///     })
/// }
/// ```
pub struct SimdJson<T>(pub T);

/// Error type for SIMD JSON extraction.
#[derive(Debug)]
pub enum SimdJsonError {
  /// Request content type is not recognized as JSON.
  InvalidContentType,
  /// Content-Type header is missing from the request.
  MissingContentType,
  /// Failed to read the request body.
  BodyReadError(String),
  /// Failed to deserialize JSON using SIMD parser.
  DeserializationError(String),
}

impl Responder for SimdJsonError {
  /// Converts the error into an HTTP response.
  fn into_response(self) -> Response {
    match self {
      SimdJsonError::InvalidContentType => (
        StatusCode::BAD_REQUEST,
        "Invalid content type; expected JSON",
      )
        .into_response(),
      SimdJsonError::MissingContentType => {
        (StatusCode::BAD_REQUEST, "Missing content type header").into_response()
      }
      SimdJsonError::BodyReadError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Failed to read request body: {}", err),
      )
        .into_response(),
      SimdJsonError::DeserializationError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Failed to deserialize JSON: {}", err),
      )
        .into_response(),
    }
  }
}

/// Returns `true` when the `Content-Type` header denotes JSON.
fn is_json_content_type(headers: &HeaderMap) -> bool {
  headers
    .get(header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .and_then(|ct| ct.parse::<mime_guess::Mime>().ok())
    .map(|mime| {
      mime.type_() == "application"
        && (mime.subtype() == "json" || mime.suffix().is_some_and(|s| s == "json"))
    })
    .unwrap_or(false)
}

impl<'a, T> FromRequest<'a> for SimdJson<T>
where
  T: DeserializeOwned + Send + 'static,
{
  type Error = SimdJsonError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      // Basic content-type validation so we can fail fast.
      if !is_json_content_type(req.headers()) {
        return Err(SimdJsonError::InvalidContentType);
      }

      // Collect the entire request body.
      let bytes = req
        .body_mut()
        .collect()
        .await
        .map_err(|e| SimdJsonError::BodyReadError(e.to_string()))?
        .to_bytes();

      let mut owned = bytes.to_vec();

      // SIMD-accelerated deserialization.
      let data = simd_json::from_slice::<T>(&mut owned)
        .map_err(|e| SimdJsonError::DeserializationError(e.to_string()))?;

      Ok(SimdJson(data))
    }
  }
}

impl<T> Responder for SimdJson<T>
where
  T: Serialize,
{
  /// Converts the wrapped data into an HTTP JSON response.
  fn into_response(self) -> Response {
    match simd_json::to_vec(&self.0) {
      Ok(buf) => {
        let mut res = Response::new(TakoBody::from(buf));
        res.headers_mut().insert(
          header::CONTENT_TYPE,
          HeaderValue::from_static(mime::APPLICATION_JSON.as_ref()),
        );
        res
      }
      Err(err) => {
        let mut res = Response::new(TakoBody::from(err.to_string()));
        *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        res.headers_mut().insert(
          header::CONTENT_TYPE,
          HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref()),
        );
        res
      }
    }
  }
}
