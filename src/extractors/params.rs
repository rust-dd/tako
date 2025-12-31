//! Path parameter extraction and deserialization for dynamic route segments.
//!
//! This module provides extractors for parsing path parameters from dynamic route segments
//! into strongly-typed Rust structures. It handles parameter extraction from routes like
//! `/users/{id}` or `/posts/{post_id}/comments/{comment_id}` and automatically deserializes
//! them using serde. The extractor supports type coercion for common types like integers,
//! floats, and strings, making it easy to work with typed path parameters in handlers.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::params::Params;
//! use tako::extractors::FromRequest;
//! use tako::types::Request;
//! use serde::Deserialize;
//!
//! #[derive(Debug, Deserialize)]
//! struct UserParams {
//!     id: u64,
//!     name: String,
//! }
//!
//! // For route: /users/{id}/profile/{name}
//! async fn user_profile(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
//!     let params: Params<UserParams> = Params::from_request(&mut req).await?;
//!
//!     Ok(format!("User ID: {}, Name: {}", params.0.id, params.0.name))
//! }
//!
//! // Simple single parameter extraction
//! #[derive(Deserialize)]
//! struct IdParam {
//!     id: u32,
//! }
//!
//! async fn get_item(params: Params<IdParam>) -> String {
//!     format!("Item ID: {}", params.0.id)
//! }
//! ```

use std::collections::HashMap;

use http::StatusCode;
use serde::de::DeserializeOwned;
use serde_json::Map;
use serde_json::Value;

use crate::extractors::FromRequest;
use crate::responder::Responder;
use crate::types::BuildHasher;
use crate::types::Request;

/// Internal helper struct for storing path parameters extracted from routes.
#[derive(Clone, Default)]
pub(crate) struct PathParams(pub HashMap<String, String, BuildHasher>);

/// Path parameter extractor with automatic deserialization to typed structures.
#[doc(alias = "params")]
pub struct Params<T>(pub T);

/// Error types for path parameter extraction and deserialization.
#[derive(Debug)]
pub enum ParamsError {
  /// Path parameters not found in request extensions (internal routing error).
  MissingPathParams,
  /// Parameter deserialization failed (type mismatch, missing field, etc.).
  DeserializationError(String),
}

impl Responder for ParamsError {
  /// Converts path parameter errors into appropriate HTTP error responses.
  fn into_response(self) -> crate::types::Response {
    match self {
      ParamsError::MissingPathParams => (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Path parameters not found in request extensions",
      )
        .into_response(),
      ParamsError::DeserializationError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Failed to deserialize path parameters: {err}"),
      )
        .into_response(),
    }
  }
}

impl<'a, T> FromRequest<'a> for Params<T>
where
  T: DeserializeOwned + Send + 'a,
{
  type Error = ParamsError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_params(req))
  }
}

impl<T> Params<T>
where
  T: DeserializeOwned,
{
  /// Extracts and deserializes path parameters from the request.
  fn extract_params(req: &Request) -> Result<Params<T>, ParamsError> {
    let path_params = req
      .extensions()
      .get::<PathParams>()
      .ok_or(ParamsError::MissingPathParams)?;

    let coerced = Self::coerce_params(&path_params.0);
    let value = Value::Object(coerced);
    let parsed = serde_json::from_value::<T>(value)
      .map_err(|e| ParamsError::DeserializationError(e.to_string()))?;

    Ok(Params(parsed))
  }

  /// Converts string parameters into JSON-compatible values with type coercion.
  fn coerce_params(map: &HashMap<String, String, BuildHasher>) -> Map<String, Value> {
    let mut result = Map::new();

    for (k, v) in map {
      let val = if let Ok(n) = v.parse::<i64>() {
        Value::Number(n.into())
      } else if let Ok(n) = v.parse::<u64>() {
        Value::Number(n.into())
      } else if let Ok(n) = v.parse::<f64>() {
        Value::Number(serde_json::Number::from_f64(n).unwrap_or_else(|| 0.into()))
      } else {
        Value::String(v.clone())
      };

      result.insert(k.clone(), val);
    }

    result
  }
}
