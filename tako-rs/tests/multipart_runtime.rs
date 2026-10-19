#![cfg(feature = "multipart")]

use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use serde::Deserialize;
use tako::body::TakoBody;
use tako::extractors::FromRequest;
use tako::extractors::multipart::BufferedUploadedFile;
use tako::extractors::multipart::InMemoryFile;
use tako::extractors::multipart::MultipartConfig;
use tako::extractors::multipart::TakoTypedMultipart;
use tako::extractors::multipart::TypedMultipartError;
use tako::extractors::multipart::UploadedFile;
use tako::types::Request;

fn request(body: impl Into<TakoBody>) -> Request {
  http::Request::builder()
    .header("content-type", "multipart/form-data; boundary=probe")
    .body(body.into())
    .unwrap()
}

fn file_request() -> Request {
  request(
    "--probe\r\nContent-Disposition: form-data; name=\"file\"; filename=\"hello.txt\"\r\n\
     Content-Type: text/plain\r\n\r\nhello\r\n--probe--\r\n",
  )
}

#[derive(Deserialize)]
struct Upload {
  file: UploadedFile,
}

async fn upload() -> UploadedFile {
  let mut request = file_request();
  let TakoTypedMultipart { data, .. } =
    TakoTypedMultipart::<Upload, UploadedFile>::from_request(&mut request)
      .await
      .unwrap();
  data.file
}

async fn moved_upload_keeps_its_file() {
  let file = upload().await;
  assert_eq!(std::fs::read(&file.path).unwrap(), b"hello");
  assert_eq!(file.size, 5);
  assert_eq!(file.file_name.as_deref(), Some("hello.txt"));
  assert_eq!(file.content_type.as_deref(), Some("text/plain"));
  let path = file.path.clone();
  drop(file);
  assert!(
    !path.exists(),
    "dropping the moved upload must remove its file"
  );
}

async fn spilled_upload_keeps_its_file() {
  #[derive(Deserialize)]
  struct BufferedUpload {
    file: BufferedUploadedFile,
  }

  let mut request = file_request();
  request
    .extensions_mut()
    .insert(MultipartConfig::new().disk_spill_threshold(2));
  let data = TakoTypedMultipart::<BufferedUpload, BufferedUploadedFile>::from_request(&mut request)
    .await
    .unwrap()
    .data;
  let BufferedUploadedFile::Disk(file) = data.file else {
    panic!("upload exceeded the spill threshold");
  };
  assert_eq!(std::fs::read(&file.path).unwrap(), b"hello");
  let path = file.path.clone();
  drop(file);
  assert!(!path.exists());
}

async fn completed_field_with_timeout() {
  #[derive(Deserialize)]
  struct Fields {
    title: String,
  }

  let mut request = request(
    "--probe\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nhello\r\n--probe--\r\n",
  );
  request
    .extensions_mut()
    .insert(MultipartConfig::new().field_chunk_timeout(Duration::from_secs(1)));
  let data = TakoTypedMultipart::<Fields, InMemoryFile>::from_request(&mut request)
    .await
    .unwrap()
    .data;
  assert_eq!(data.title, "hello");
}

async fn stalled_fields_time_out() {
  for prefix in [
    "",
    "--probe\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nhello",
    "--probe\r\nContent-Disposition: form-data; name=\"file\"; filename=\"hello.txt\"\r\n\r\nhello",
  ] {
    let stream = futures_util::stream::iter([Ok::<_, std::io::Error>(Bytes::from(prefix))])
      .chain(futures_util::stream::pending());
    let mut request = request(TakoBody::from_stream(stream));
    request
      .extensions_mut()
      .insert(MultipartConfig::new().field_chunk_timeout(Duration::from_millis(10)));
    let error = TakoTypedMultipart::<serde_json::Value, InMemoryFile>::from_request(&mut request)
      .await
      .err()
      .expect("a stalled multipart read must time out");
    assert!(
      matches!(error, TypedMultipartError::FieldError(message) if message.contains("timeout"))
    );
  }
}

async fn persist_and_disarm_keep_files() {
  let mut file = upload().await;
  let original = file.path.clone();
  let destination = original.with_extension("saved");
  file.persist(destination.clone()).await.unwrap();
  assert_eq!(file.path, destination);
  assert!(!original.exists());
  drop(file);
  assert_eq!(std::fs::read(&destination).unwrap(), b"hello");
  std::fs::remove_file(destination).unwrap();

  let mut file = upload().await;
  let path = file.path.clone();
  file.disarm_cleanup();
  drop(file);
  assert_eq!(std::fs::read(&path).unwrap(), b"hello");
  std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn tokio_moved_upload_keeps_its_file() {
  moved_upload_keeps_its_file().await;
}

#[tokio::test]
async fn tokio_spilled_upload_keeps_its_file() {
  spilled_upload_keeps_its_file().await;
}

#[tokio::test]
async fn tokio_completed_field_with_timeout() {
  completed_field_with_timeout().await;
}

#[tokio::test]
async fn tokio_stalled_fields_time_out() {
  tokio::time::timeout(Duration::from_secs(2), stalled_fields_time_out())
    .await
    .expect("multipart timeout failed to fire");
}

#[tokio::test]
async fn tokio_persist_and_disarm_keep_files() {
  persist_and_disarm_keep_files().await;
}

#[cfg(feature = "compio")]
#[compio::test]
async fn compio_moved_upload_keeps_its_file() {
  moved_upload_keeps_its_file().await;
}

#[cfg(feature = "compio")]
#[compio::test]
async fn compio_spilled_upload_keeps_its_file() {
  spilled_upload_keeps_its_file().await;
}

#[cfg(feature = "compio")]
#[compio::test]
async fn compio_completed_field_with_timeout() {
  completed_field_with_timeout().await;
}

#[cfg(feature = "compio")]
#[compio::test]
async fn compio_stalled_fields_time_out() {
  compio::time::timeout(Duration::from_secs(2), stalled_fields_time_out())
    .await
    .expect("multipart timeout failed to fire");
}

#[cfg(feature = "compio")]
#[compio::test]
async fn compio_persist_and_disarm_keep_files() {
  persist_and_disarm_keep_files().await;
}
