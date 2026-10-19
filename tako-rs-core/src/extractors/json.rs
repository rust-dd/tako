//! The [`Json`] body extractor and responder.
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
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::body::TakoBody;
use crate::extractors::Entries;
use crate::extractors::FromRequest;
use crate::extractors::body::collect_body;
use crate::extractors::is_json_content_type;
use crate::responder::Responder;
use crate::types::Request;
use crate::types::Response;

/// When `Json<T>` parses with `sonic-rs` (feature `simd-sonic`), set per route by
/// [`Route::simd_json`](crate::route::Route::simd_json); prefer `Never` for untrusted input.
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
  /// Use the SIMD parser for bodies of at least this many bytes; the default is 2 MiB.
  Threshold(usize),
}

impl Default for SimdJsonMode {
  fn default() -> Self {
    Self::Threshold(2 * 1024 * 1024)
  }
}

/// Deserializes a JSON request body, or serializes `T` as a JSON response.
#[doc(alias = "json")]
pub struct Json<T>(pub T);

/// Why [`Json`] extraction failed; answered with 400, or 413 for an oversized body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonError {
  /// Content-Type header is not application/json or compatible JSON type.
  InvalidContentType,
  /// The body could not be buffered, for example because it exceeds the limit.
  BodyReadError(crate::extractors::body::BodyReadError),
  /// JSON deserialization failed (syntax error, type mismatch, etc.).
  DeserializationError(String),
}

impl std::fmt::Display for JsonError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::InvalidContentType => {
        write!(f, "invalid content type; expected application/json")
      }
      Self::BodyReadError(err) => write!(f, "failed to read request body: {err}"),
      Self::DeserializationError(err) => write!(f, "failed to deserialize JSON: {err}"),
    }
  }
}

impl std::error::Error for JsonError {}

impl Responder for JsonError {
  fn into_response(self) -> crate::types::Response {
    match self {
      JsonError::InvalidContentType => (
        StatusCode::BAD_REQUEST,
        "Invalid content type; expected application/json",
      )
        .into_response(),
      JsonError::BodyReadError(err) => {
        (err.status(), format!("Failed to read request body: {err}")).into_response()
      }
      JsonError::DeserializationError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Failed to deserialize JSON: {err}"),
      )
        .into_response(),
    }
  }
}

impl<'a, T> FromRequest<'a> for Json<T>
where
  T: DeserializeOwned + Send + 'static,
{
  type Error = JsonError;
  const ENTRIES: Entries = Entries::BODY_LIMIT.union(Entries::SIMD_JSON);

  /// Requires a JSON `Content-Type` and a non-empty body.
  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      if !is_json_content_type(req.headers()) {
        return Err(JsonError::InvalidContentType);
      }

      let body_bytes = collect_body(req).await.map_err(JsonError::BodyReadError)?;

      if body_bytes.is_empty() {
        return Err(JsonError::DeserializationError(
          "request body is empty, expected JSON payload".to_string(),
        ));
      }

      #[cfg(feature = "simd-sonic")]
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
          sonic_rs::from_slice::<T>(&body_bytes)
            .map_err(|e| JsonError::DeserializationError(e.to_string()))?
        } else {
          serde_json::from_slice(&body_bytes)
            .map_err(|e| JsonError::DeserializationError(e.to_string()))?
        }
      };
      #[cfg(not(feature = "simd-sonic"))]
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
