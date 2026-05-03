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
use crate::extractors::FromRequestParts;
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
    futures_util::future::ready(Self::extract_params(req.extensions()))
  }
}

impl<'a, T> FromRequestParts<'a> for Params<T>
where
  T: DeserializeOwned + Send + 'a,
{
  type Error = ParamsError;

  fn from_request_parts(
    parts: &'a mut http::request::Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_params(&parts.extensions))
  }
}

impl<T> Params<T>
where
  T: DeserializeOwned,
{
  /// Extracts and deserializes path parameters from request extensions.
  fn extract_params(extensions: &http::Extensions) -> Result<Params<T>, ParamsError> {
    let path_params = extensions
      .get::<PathParams>()
      .ok_or(ParamsError::MissingPathParams)?;

    let parsed = T::deserialize(PathParamsDeserializer(&path_params.0))
      .map_err(|e| ParamsError::DeserializationError(e.to_string()))?;

    Ok(Params(parsed))
  }
}

#[cfg(test)]
mod tests {
  use serde::Deserialize;

  use super::*;

  fn deserialize<T: DeserializeOwned>(slots: &[(&str, &str)]) -> Result<T, PathParamsDeError> {
    let owned: SmallVec<[(String, String); 4]> = slots
      .iter()
      .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
      .collect();
    T::deserialize(PathParamsDeserializer(&owned))
  }

  #[derive(Debug, Deserialize, PartialEq)]
  struct UserParams {
    id: u64,
    name: String,
  }

  #[test]
  fn deserialize_struct_with_typed_fields() {
    let value: UserParams = deserialize(&[("id", "42"), ("name", "alice")]).unwrap();
    assert_eq!(
      value,
      UserParams {
        id: 42,
        name: "alice".to_string(),
      }
    );
  }

  #[test]
  fn deserialize_struct_reports_missing_field() {
    let err = deserialize::<UserParams>(&[("id", "42")]).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("name") || msg.contains("missing"));
  }

  #[test]
  fn deserialize_single_primitive_via_one_slot() {
    let value: u64 = deserialize(&[("id", "1234")]).unwrap();
    assert_eq!(value, 1234);
  }

  #[test]
  fn deserialize_string_value() {
    let value: String = deserialize(&[("name", "alice")]).unwrap();
    assert_eq!(value, "alice");
  }

  #[test]
  fn deserialize_tuple_two_slots() {
    let value: (u64, String) = deserialize(&[("a", "7"), ("b", "x")]).unwrap();
    assert_eq!(value, (7, "x".to_string()));
  }

  #[test]
  fn deserialize_vec_string() {
    let value: Vec<String> = deserialize(&[("a", "1"), ("b", "2"), ("c", "3")]).unwrap();
    assert_eq!(
      value,
      vec!["1".to_string(), "2".to_string(), "3".to_string()]
    );
  }

  #[test]
  fn deserialize_option_present() {
    let value: Option<u64> = deserialize(&[("id", "9")]).unwrap();
    assert_eq!(value, Some(9));
  }

  #[test]
  fn deserialize_rejects_non_integer_into_u64() {
    let err = deserialize::<u64>(&[("id", "not_a_number")]).unwrap_err();
    assert!(!err.to_string().is_empty());
  }

  #[test]
  fn extract_params_returns_missing_when_extension_absent() {
    let extensions = http::Extensions::new();
    match Params::<UserParams>::extract_params(&extensions) {
      Err(e) => assert_eq!(e, ParamsError::MissingPathParams),
      Ok(_) => panic!("expected MissingPathParams"),
    }
  }

  #[test]
  fn extract_params_returns_value_when_extension_present() {
    let mut extensions = http::Extensions::new();
    let mut params = SmallVec::<[(String, String); 4]>::new();
    params.push(("id".to_string(), "5".to_string()));
    params.push(("name".to_string(), "bob".to_string()));
    extensions.insert(PathParams(params));

    let extracted = Params::<UserParams>::extract_params(&extensions).expect("extract ok");
    assert_eq!(
      extracted.0,
      UserParams {
        id: 5,
        name: "bob".to_string(),
      }
    );
  }

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

struct PathParamsDeserializer<'de>(&'de [(String, String)]);

impl<'de> PathParamsDeserializer<'de> {
  fn single<V: Visitor<'de>>(
    self,
    visitor: V,
    f: impl FnOnce(ValueDeserializer<'de>, V) -> Result<V::Value, PathParamsDeError>,
  ) -> Result<V::Value, PathParamsDeError> {
    if self.0.len() != 1 {
      return Err(de::Error::custom(format!(
        "expected exactly 1 path parameter, got {}",
        self.0.len()
      )));
    }
    f(ValueDeserializer(&self.0[0].1), visitor)
  }
}

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

  fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    visitor.visit_seq(PathParamsSeqAccess {
      params: self.0,
      index: 0,
    })
  }

  fn deserialize_tuple<V: Visitor<'de>>(
    self,
    len: usize,
    visitor: V,
  ) -> Result<V::Value, Self::Error> {
    if self.0.len() != len {
      return Err(de::Error::custom(format!(
        "expected tuple of {} path parameters, got {}",
        len,
        self.0.len()
      )));
    }
    self.deserialize_seq(visitor)
  }

  fn deserialize_tuple_struct<V: Visitor<'de>>(
    self,
    _name: &'static str,
    len: usize,
    visitor: V,
  ) -> Result<V::Value, Self::Error> {
    self.deserialize_tuple(len, visitor)
  }

  fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    if self.0.is_empty() {
      visitor.visit_none()
    } else {
      visitor.visit_some(self)
    }
  }

  fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self.single(visitor, |d, v| d.deserialize_bool(v))
  }

  fn deserialize_i8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self.single(visitor, |d, v| d.deserialize_i8(v))
  }

  fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self.single(visitor, |d, v| d.deserialize_i16(v))
  }

  fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self.single(visitor, |d, v| d.deserialize_i32(v))
  }

  fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self.single(visitor, |d, v| d.deserialize_i64(v))
  }

  fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self.single(visitor, |d, v| d.deserialize_u8(v))
  }

  fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self.single(visitor, |d, v| d.deserialize_u16(v))
  }

  fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self.single(visitor, |d, v| d.deserialize_u32(v))
  }

  fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self.single(visitor, |d, v| d.deserialize_u64(v))
  }

  fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self.single(visitor, |d, v| d.deserialize_f32(v))
  }

  fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self.single(visitor, |d, v| d.deserialize_f64(v))
  }

  fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self.single(visitor, |d, v| d.deserialize_char(v))
  }

  fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self.single(visitor, |d, v| d.deserialize_str(v))
  }

  fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self.single(visitor, |d, v| d.deserialize_string(v))
  }

  fn deserialize_enum<V: Visitor<'de>>(
    self,
    name: &'static str,
    variants: &'static [&'static str],
    visitor: V,
  ) -> Result<V::Value, Self::Error> {
    self.single(visitor, |d, v| d.deserialize_enum(name, variants, v))
  }

  fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
    self.single(visitor, |d, v| d.deserialize_identifier(v))
  }

  serde::forward_to_deserialize_any! {
    bytes byte_buf unit unit_struct ignored_any
  }
}

struct PathParamsSeqAccess<'de> {
  params: &'de [(String, String)],
  index: usize,
}

impl<'de> de::SeqAccess<'de> for PathParamsSeqAccess<'de> {
  type Error = PathParamsDeError;

  fn next_element_seed<T: de::DeserializeSeed<'de>>(
    &mut self,
    seed: T,
  ) -> Result<Option<T::Value>, Self::Error> {
    if self.index >= self.params.len() {
      return Ok(None);
    }
    let value = &self.params[self.index].1;
    self.index += 1;
    seed.deserialize(ValueDeserializer(value)).map(Some)
  }

  fn size_hint(&self) -> Option<usize> {
    Some(self.params.len() - self.index)
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
