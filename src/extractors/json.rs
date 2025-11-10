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

use http_body_util::BodyExt;
use hyper::{StatusCode, header::HeaderValue};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
  body::TakoBody,
  extractors::FromRequest,
  responder::Responder,
  types::{Request, Response},
};

/// JSON request body extractor with automatic deserialization.
pub struct Json<T>(pub T);

/// Error types for JSON extraction and deserialization.
#[derive(Debug)]
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
        format!("Failed to read request body: {}", err),
      )
        .into_response(),
      JsonError::DeserializationError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Failed to deserialize JSON: {}", err),
      )
        .into_response(),
    }
  }
}

/// Checks if the Content-Type header indicates JSON content.
fn is_json_content_type(headers: &http::HeaderMap) -> bool {
  headers
    .get(http::header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .and_then(|ct| ct.parse::<mime_guess::Mime>().ok())
    .map(|mime| {
      mime.type_() == "application"
        && (mime.subtype() == "json" || mime.suffix().is_some_and(|s| s == "json"))
    })
    .unwrap_or(false)
}

impl<'a, T> FromRequest<'a> for Json<T>
where
  T: DeserializeOwned + Send + 'static,
{
  type Error = JsonError;

  /// Extracts and deserializes JSON data from the HTTP request body.
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

      // Deserialize JSON using serde into the target type
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
