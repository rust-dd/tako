use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use super::PendingUploads;
use super::TempFileCleanup;
use crate::multipart::UploadedFile;

fn pending_upload() -> (PendingUploads, Value, PathBuf) {
  let path = std::env::temp_dir().join(format!("tako-cleanup-test-{}", Uuid::new_v4()));
  std::fs::write(&path, b"hello").unwrap();
  let mut pending = PendingUploads::default();
  pending.insert(TempFileCleanup::for_path(path.clone()));
  let value = serde_json::json!({
    "file_name": "hello.txt",
    "content_type": "text/plain",
    "path": path,
    "size": 5,
  });
  (pending, value, path)
}

#[test]
fn ignored_upload_is_removed_at_scope_end() {
  let (pending, value, path) = pending_upload();
  pending.deserialize::<Value>(value).unwrap();
  assert!(!path.exists());
}

struct Panics;

impl<'de> Deserialize<'de> for Panics {
  fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
    let file = UploadedFile::deserialize(deserializer)?;
    assert!(file.path.exists());
    panic!("intentional deserializer panic");
  }
}

#[test]
fn panic_removes_upload_and_clears_scope() {
  let (pending, value, path) = pending_upload();
  let saved = value.clone();
  assert!(std::panic::catch_unwind(|| pending.deserialize::<Panics>(value)).is_err());
  assert!(!path.exists());
  std::fs::write(&path, b"unrelated replacement").unwrap();
  drop(serde_json::from_value::<UploadedFile>(saved).unwrap());
  assert!(path.exists(), "the failed scope must not remain active");
  std::fs::remove_file(path).unwrap();
}

#[test]
fn nested_scope_restores_ownership_after_failure_and_panic() {
  struct Nested(UploadedFile);

  impl<'de> Deserialize<'de> for Nested {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
      let value = Value::deserialize(deserializer)?;
      let nested = PendingUploads::default()
        .deserialize::<UploadedFile>(value.clone())
        .map_err(serde::de::Error::custom)?;
      let outer_path = nested.path.clone();
      drop(nested);
      assert!(
        outer_path.exists(),
        "an inner scope cannot claim an outer upload"
      );

      let (pending, mut invalid, invalid_path) = pending_upload();
      invalid.as_object_mut().unwrap().remove("size");
      assert!(pending.deserialize::<UploadedFile>(invalid).is_err());
      assert!(!invalid_path.exists());
      let (pending, panic_value, panic_path) = pending_upload();
      assert!(std::panic::catch_unwind(|| pending.deserialize::<Panics>(panic_value)).is_err());
      assert!(!panic_path.exists());

      serde_json::from_value::<UploadedFile>(value)
        .map(Self)
        .map_err(serde::de::Error::custom)
    }
  }

  let (pending, value, path) = pending_upload();
  let Nested(file) = pending.deserialize::<Nested>(value).unwrap();
  assert!(path.exists());
  drop(file);
  assert!(!path.exists(), "the outer scope must have been restored");
}

#[test]
fn uploaded_file_preserves_its_serde_struct_name() {
  struct NamedUpload(Value);

  impl<'de> serde::Deserializer<'de> for NamedUpload {
    type Error = serde_json::Error;

    fn deserialize_any<V: serde::de::Visitor<'de>>(
      self,
      visitor: V,
    ) -> Result<V::Value, Self::Error> {
      self.0.deserialize_any(visitor)
    }

    fn deserialize_struct<V: serde::de::Visitor<'de>>(
      self,
      name: &'static str,
      fields: &'static [&'static str],
      visitor: V,
    ) -> Result<V::Value, Self::Error> {
      if name != "UploadedFile" {
        return Err(serde::de::Error::custom("expected UploadedFile struct"));
      }
      self.0.deserialize_struct(name, fields, visitor)
    }

    serde::forward_to_deserialize_any! {
      bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes byte_buf
      option unit unit_struct newtype_struct seq tuple tuple_struct map enum identifier ignored_any
    }
  }

  let (pending, value, path) = pending_upload();
  let file = UploadedFile::deserialize(NamedUpload(value)).unwrap();
  assert_eq!(file.path, path);
  drop(file);
  assert!(path.exists());
  drop(pending);
  assert!(!path.exists());
}
