#![cfg(feature = "multipart")]

use std::cell::RefCell;
use std::path::PathBuf;

use serde::Deserialize;
use tako::body::TakoBody;
use tako::extractors::FromRequest;
use tako::extractors::multipart::TakoTypedMultipart;
use tako::extractors::multipart::TypedMultipartError;
use tako::extractors::multipart::UploadedFile;
use tako::types::Request;

thread_local! {
  static OBSERVED_FILE: RefCell<Option<(PathBuf, bool)>> = const { RefCell::new(None) };
}

fn request() -> Request {
  http::Request::builder()
    .header("content-type", "multipart/form-data; boundary=probe")
    .body(TakoBody::from(
      "--probe\r\nContent-Disposition: form-data; name=\"file\"; filename=\"hello.txt\"\r\n\
       Content-Type: text/plain\r\n\r\nhello\r\n--probe--\r\n",
    ))
    .unwrap()
}

fn observe_file<'de, D: serde::Deserializer<'de>>(
  deserializer: D,
) -> Result<UploadedFile, D::Error> {
  let file = UploadedFile::deserialize(deserializer)?;
  OBSERVED_FILE.with(|observed| {
    *observed.borrow_mut() = Some((file.path.clone(), file.path.exists()));
  });
  Ok(file)
}

#[tokio::test]
async fn deserialization_error_cleans_up_uploaded_file() {
  #[derive(Deserialize)]
  struct InvalidForm {
    #[serde(deserialize_with = "observe_file", rename = "file")]
    _file: UploadedFile,
    _missing: String,
  }

  let mut request = request();
  let result = TakoTypedMultipart::<InvalidForm, UploadedFile>::from_request(&mut request).await;
  assert!(matches!(
    result,
    Err(TypedMultipartError::DeserializationError(_))
  ));
  let (path, existed) = OBSERVED_FILE.with(|observed| observed.take().unwrap());
  assert!(existed, "the file must survive until deserialization");
  assert!(
    !path.exists(),
    "failed deserialization must remove the file"
  );
}

#[tokio::test]
async fn failed_persist_keeps_cleanup_armed() {
  #[derive(Deserialize)]
  struct Form {
    file: UploadedFile,
  }

  let mut file = TakoTypedMultipart::<Form, UploadedFile>::from_request(&mut request())
    .await
    .unwrap()
    .data
    .file;
  let original = file.path.clone();
  let missing_parent = original.with_extension("missing").join("saved");
  assert!(file.persist(missing_parent).await.is_err());
  assert_eq!(file.path, original);
  assert_eq!(std::fs::read(&original).unwrap(), b"hello");
  drop(file);
  assert!(!original.exists());
}

#[test]
fn client_deserialized_paths_stay_disarmed() {
  let path = std::env::temp_dir().join(format!("tako-client-upload-{}", std::process::id()));
  std::fs::write(&path, b"keep me").unwrap();
  let value = serde_json::json!({
    "file_name": "untrusted.txt",
    "content_type": "text/plain",
    "path": path,
    "size": 7,
    "cleanup": { "path": path },
  });
  let file = serde_json::from_value::<UploadedFile>(value).unwrap();
  drop(file);
  assert_eq!(std::fs::read(&path).unwrap(), b"keep me");
  std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn untagged_form_retry_keeps_upload_alive() {
  #[derive(Deserialize)]
  #[serde(untagged)]
  enum Form {
    RequiresTitle {
      #[serde(rename = "file")]
      _file: UploadedFile,
      _missing: String,
    },
    Upload {
      file: UploadedFile,
    },
  }

  let data = TakoTypedMultipart::<Form, UploadedFile>::from_request(&mut request())
    .await
    .unwrap()
    .data;
  let Form::Upload { file } = data else {
    panic!("the first enum variant requires an absent field");
  };
  assert_eq!(std::fs::read(&file.path).unwrap(), b"hello");
  let path = file.path.clone();
  drop(file);
  assert!(!path.exists());
}

#[tokio::test]
async fn serialized_upload_does_not_share_cleanup_outside_extraction() {
  #[derive(Deserialize)]
  struct Form {
    file: UploadedFile,
  }

  let file = TakoTypedMultipart::<Form, UploadedFile>::from_request(&mut request())
    .await
    .unwrap()
    .data
    .file;
  let copy = serde_json::from_value::<UploadedFile>(serde_json::to_value(&file).unwrap()).unwrap();
  drop(copy);
  assert_eq!(std::fs::read(&file.path).unwrap(), b"hello");
  let path = file.path.clone();
  drop(file);
  assert!(!path.exists());
}
