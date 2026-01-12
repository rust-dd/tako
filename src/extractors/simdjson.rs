#![cfg_attr(docsrs, doc(cfg(feature = "simd")))]
//! SIMD-accelerated JSON extraction from HTTP request bodies.
//!
//! This module provides two extractors behind the `simd` feature:
//! - [`SimdJson`](crate::extractors::simdjson::SimdJson) uses the `simd_json` crate.
//! - [`SonicJson`](crate::extractors::simdjson::SonicJson) uses the `sonic_rs` crate.
//!
//! Both extractors validate the `Content-Type` header, read the full request body into an owned buffer,
//! and deserialize it into the requested type.
//!
//! # Examples
//!
//! Using [`SimdJson`](crate::extractors::simdjson::SimdJson):
//!
//! ```
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
//!
//! Using [`SonicJson`](crate::extractors::simdjson::SonicJson):
//!
//! ```
//! use tako::extractors::simdjson::SonicJson;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Deserialize, Serialize)]
//! struct User {
//!     name: String,
//!     email: String,
//!     age: u32,
//! }
//!
//! async fn create_user_handler(SonicJson(user): SonicJson<User>) -> SonicJson<User> {
//!     println!("Creating user: {}", user.name);
//!     // Process user creation...
//!     SonicJson(user)
//! }
//! ```

use http::HeaderMap;
use http::StatusCode;
use http::header::HeaderValue;
use http::header::{self};
use http_body_util::BodyExt;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::body::TakoBody;
use crate::extractors::FromRequest;
use crate::responder::Responder;
use crate::types::Request;
use crate::types::Response;

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
/// See also [`SonicJson`] for an alternative backend with the same API.
///
/// # Examples
///
/// ```
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
#[doc(alias = "simdjson")]
pub struct SimdJson<T>(pub T);

/// Error type for the SIMD JSON extractors.
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

/// An extractor that (de)serializes JSON using the `sonic_rs` backend.
///
/// `SonicJson<T>` behaves similarly to [`SimdJson`] but uses `sonic_rs` for parsing
/// and serialization.
///
/// The extractor validates the `Content-Type` header, reads the full request body,
/// and deserializes it into the requested type.
///
/// The extractor also implements [`Responder`], allowing it to be returned
/// directly from handler functions for JSON responses.
///
/// # Examples
///
/// ```
/// use tako::extractors::simdjson::SonicJson;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Deserialize, Serialize)]
/// struct ApiResponse {
///     ok: bool,
/// }
///
/// async fn api_handler(SonicJson(_request): SonicJson<ApiResponse>) -> SonicJson<ApiResponse> {
///     SonicJson(ApiResponse { ok: true })
/// }
/// ```
#[doc(alias = "sonicjson")]
pub struct SonicJson<T>(pub T);

impl<'a, T> FromRequest<'a> for SonicJson<T>
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
      let data = sonic_rs::from_slice::<T>(&mut owned)
        .map_err(|e| SimdJsonError::DeserializationError(e.to_string()))?;

      Ok(SonicJson(data))
    }
  }
}

impl<T> Responder for SonicJson<T>
where
  T: Serialize,
{
  /// Converts the wrapped data into an HTTP JSON response.
  fn into_response(self) -> Response {
    match sonic_rs::to_vec(&self.0) {
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
