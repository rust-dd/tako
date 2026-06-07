//! Serde `Deserializer` over the captured path-parameter slot list, plus the
//! map/seq access adapters that drive struct, tuple and sequence shapes.

use serde::de::Deserializer;
use serde::de::MapAccess;
use serde::de::Visitor;
use serde::de::{self};

use super::decode::ValueDeserializer;
use super::error::PathParamsDeError;

pub(crate) struct PathParamsDeserializer<'de>(pub(crate) &'de [(String, String)]);

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

// Trampoline closures preserve the `V` visitor binding through the trait
// method; replacing them with `Trait::method` function refs introduces UFCS
// ambiguity around the generic `V`.
#[allow(clippy::redundant_closure_for_method_calls)]
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
      // Include the captured parameter names so the resulting error points
      // the user at the actual mismatch between the route pattern (e.g.
      // `/users/{id}/posts/{post_id}` — 2 slots) and the tuple type they
      // tried to extract.
      let captured: Vec<&str> = self.0.iter().map(|(k, _)| k.as_str()).collect();
      return Err(de::Error::custom(format!(
        "expected tuple of {} path parameters, got {} (captured: [{}])",
        len,
        self.0.len(),
        captured.join(", ")
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

#[cfg(test)]
mod tests {
  use serde::Deserialize;
  use serde::de::DeserializeOwned;
  use smallvec::SmallVec;

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
}
