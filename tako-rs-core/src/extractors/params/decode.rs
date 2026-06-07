//! Per-value string deserializer that coerces a captured segment into the
//! type the visitor asks for.

use serde::de::Deserializer;
use serde::de::Visitor;
use serde::de::{self};

use super::error::PathParamsDeError;

/// Deserializer for a single string value that attempts numeric coercion
/// only when the visitor requests a numeric type.
pub(crate) struct ValueDeserializer<'de>(pub(crate) &'de str);

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
