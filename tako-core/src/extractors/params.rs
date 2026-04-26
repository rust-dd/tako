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

use std::fmt;

use http::StatusCode;
use serde::de::DeserializeOwned;
use serde::de::Deserializer;
use serde::de::MapAccess;
use serde::de::Visitor;
use serde::de::{self};
use smallvec::SmallVec;

use crate::extractors::FromRequest;
use crate::responder::Responder;
use crate::types::Request;

/// Internal helper struct for storing path parameters extracted from routes.
#[derive(Clone, Default)]
#[doc(hidden)]
pub struct PathParams(pub SmallVec<[(String, String); 4]>);

/// Path parameter extractor with automatic deserialization to typed structures.
#[doc(alias = "params")]
pub struct Params<T>(pub T);

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

    let parsed = T::deserialize(PathParamsDeserializer(&path_params.0))
      .map_err(|e| ParamsError::DeserializationError(e.to_string()))?;

    Ok(Params(parsed))
  }
}

struct PathParamsDeserializer<'de>(&'de [(String, String)]);

impl<'de> Deserializer<'de> for PathParamsDeserializer<'de> {
  type Error = PathParamsDeError;

  fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self.deserialize_map(visitor)
  }

  fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    visitor.visit_map(PathParamsMapAccess {
      params: self.0,
      index: 0,
      value: None,
    })
  }

  fn deserialize_struct<V: Visitor<'de>>(
    self,
    _name: &'static str,
    _fields: &'static [&'static str],
    visitor: V,
  ) -> Result<V::Value, Self::Error> {
    self.deserialize_map(visitor)
  }

  // For single-value extraction (e.g. Params<String> or Params<u64>)
  fn deserialize_newtype_struct<V: Visitor<'de>>(
    self,
    _name: &'static str,
    visitor: V,
  ) -> Result<V::Value, Self::Error> {
    if self.0.len() == 1 {
      visitor.visit_newtype_struct(ValueDeserializer(&self.0[0].1))
    } else {
      self.deserialize_map(visitor)
    }
  }

  serde::forward_to_deserialize_any! {
    bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes
    byte_buf option unit unit_struct seq tuple tuple_struct enum identifier
    ignored_any
  }
}

struct PathParamsMapAccess<'de> {
  params: &'de [(String, String)],
  index: usize,
  value: Option<&'de str>,
}

impl<'de> MapAccess<'de> for PathParamsMapAccess<'de> {
  type Error = PathParamsDeError;

  fn next_key_seed<K: de::DeserializeSeed<'de>>(
    &mut self,
    seed: K,
  ) -> Result<Option<K::Value>, Self::Error> {
    if self.index >= self.params.len() {
      return Ok(None);
    }
    let (ref key, ref value) = self.params[self.index];
    self.value = Some(value.as_str());
    self.index += 1;
    seed.deserialize(ValueDeserializer(key.as_str())).map(Some)
  }

  fn next_value_seed<V: de::DeserializeSeed<'de>>(
    &mut self,
    seed: V,
  ) -> Result<V::Value, Self::Error> {
    let value = self
      .value
      .take()
      .expect("next_value_seed called before next_key_seed");
    seed.deserialize(ValueDeserializer(value))
  }
}

/// Deserializer for a single string value that attempts numeric coercion
/// only when the visitor requests a numeric type.
struct ValueDeserializer<'de>(&'de str);

impl<'de> Deserializer<'de> for ValueDeserializer<'de> {
  type Error = PathParamsDeError;

  fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    visitor.visit_borrowed_str(self.0)
  }

  fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    match self.0 {
      "true" | "1" => visitor.visit_bool(true),
      "false" | "0" => visitor.visit_bool(false),
      _ => Err(de::Error::custom(format!(
        "cannot parse '{}' as bool",
        self.0
      ))),
    }
  }

  fn deserialize_i8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self
      .0
      .parse::<i8>()
      .map_err(de::Error::custom)
      .and_then(|v| visitor.visit_i8(v))
  }

  fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self
      .0
      .parse::<i16>()
      .map_err(de::Error::custom)
      .and_then(|v| visitor.visit_i16(v))
  }

  fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self
      .0
      .parse::<i32>()
      .map_err(de::Error::custom)
      .and_then(|v| visitor.visit_i32(v))
  }

  fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self
      .0
      .parse::<i64>()
      .map_err(de::Error::custom)
      .and_then(|v| visitor.visit_i64(v))
  }

  fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self
      .0
      .parse::<u8>()
      .map_err(de::Error::custom)
      .and_then(|v| visitor.visit_u8(v))
  }

  fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self
      .0
      .parse::<u16>()
      .map_err(de::Error::custom)
      .and_then(|v| visitor.visit_u16(v))
  }

  fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self
      .0
      .parse::<u32>()
      .map_err(de::Error::custom)
      .and_then(|v| visitor.visit_u32(v))
  }

  fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self
      .0
      .parse::<u64>()
      .map_err(de::Error::custom)
      .and_then(|v| visitor.visit_u64(v))
  }

  fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self
      .0
      .parse::<f32>()
      .map_err(de::Error::custom)
      .and_then(|v| visitor.visit_f32(v))
  }

  fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self
      .0
      .parse::<f64>()
      .map_err(de::Error::custom)
      .and_then(|v| visitor.visit_f64(v))
  }

  fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    let mut chars = self.0.chars();
    match (chars.next(), chars.next()) {
      (Some(c), None) => visitor.visit_char(c),
      _ => Err(de::Error::custom(format!(
        "cannot parse '{}' as char",
        self.0
      ))),
    }
  }

  fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    visitor.visit_borrowed_str(self.0)
  }

  fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    visitor.visit_string(self.0.to_owned())
  }

  fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    visitor.visit_some(self)
  }

  fn deserialize_newtype_struct<V: Visitor<'de>>(
    self,
    _name: &'static str,
    visitor: V,
  ) -> Result<V::Value, Self::Error> {
    visitor.visit_newtype_struct(self)
  }

  fn deserialize_enum<V: Visitor<'de>>(
    self,
    _name: &'static str,
    _variants: &'static [&'static str],
    visitor: V,
  ) -> Result<V::Value, Self::Error> {
    visitor.visit_enum(de::value::StrDeserializer::new(self.0))
  }

  fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    visitor.visit_borrowed_str(self.0)
  }

  serde::forward_to_deserialize_any! {
    bytes byte_buf unit unit_struct seq tuple tuple_struct map struct
    ignored_any
  }
}

// Custom error type for the deserializer
#[derive(Debug)]
struct PathParamsDeError(String);

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
