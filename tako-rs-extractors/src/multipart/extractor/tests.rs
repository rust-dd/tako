use std::cell::RefCell;
use std::path::PathBuf;
use std::task::Poll;
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use serde::Deserialize;
use serde::Serialize;
use tako_rs_core::body::TakoBody;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::types::Request;

use crate::multipart::FromMultipartField;
use crate::multipart::MultipartConfig;
use crate::multipart::TakoTypedMultipart;
use crate::multipart::TempFileCleanup;
use crate::multipart::TypedMultipartError;
use crate::multipart::UploadedFile;

thread_local! {
  static OBSERVED_PATH: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

#[derive(Serialize)]
#[serde(transparent)]
struct ObservedFile(UploadedFile);

impl FromMultipartField for ObservedFile {
  async fn from_field(field: multer::Field<'_>) -> anyhow::Result<Self> {
    let file = UploadedFile::from_field(field).await?;
    OBSERVED_PATH.with(|path| *path.borrow_mut() = Some(file.path.clone()));
    Ok(Self(file))
  }

  fn take_cleanup(&mut self) -> TempFileCleanup {
    self.0.take_cleanup()
  }
}

#[derive(Serialize)]
#[serde(transparent)]
struct SlowFile(UploadedFile);

impl FromMultipartField for SlowFile {
  async fn from_field(field: multer::Field<'_>) -> anyhow::Result<Self> {
    let ObservedFile(file) = ObservedFile::from_field(field).await?;
    std::future::pending::<()>().await;
    Ok(Self(file))
  }
}

fn body_prefix() -> &'static str {
  "--probe\r\nContent-Disposition: form-data; name=\"file\"; filename=\"hello.txt\"\r\n\r\nhello\r\n"
}

fn request(body: impl Into<TakoBody>) -> Request {
  http::Request::builder()
    .header("content-type", "multipart/form-data; boundary=probe")
    .body(body.into())
    .unwrap()
}

fn observed_path() -> PathBuf {
  OBSERVED_PATH.with(|path| path.take().expect("the file reader must have completed"))
}

async fn cancel_after_upload() {
  let body = format!("{}--probe\r\n", body_prefix());
  let stream = futures_util::stream::iter([Ok::<_, std::io::Error>(Bytes::from(body))])
    .chain(futures_util::stream::pending());
  let mut request = request(TakoBody::from_stream(stream));
  let mut extraction =
    Box::pin(TakoTypedMultipart::<serde_json::Value, ObservedFile>::from_request(&mut request));
  let path = std::future::poll_fn(|context| {
    assert!(extraction.as_mut().poll(context).is_pending());
    match OBSERVED_PATH.with(RefCell::take) {
      Some(path) => Poll::Ready(path),
      None => Poll::Pending,
    }
  })
  .await;
  assert_eq!(std::fs::read(&path).unwrap(), b"hello");
  drop(extraction);
  assert!(
    !path.exists(),
    "cancelling the request must remove its completed upload"
  );
}

async fn timed_out_reader_cleans_upload() {
  let mut request = request(format!("{}--probe--\r\n", body_prefix()));
  request
    .extensions_mut()
    .insert(MultipartConfig::new().field_chunk_timeout(Duration::from_millis(100)));
  let result = TakoTypedMultipart::<serde_json::Value, SlowFile>::from_request(&mut request).await;
  assert!(
    matches!(result, Err(TypedMultipartError::FieldError(message)) if message.contains("timeout"))
  );
  assert!(!observed_path().exists());
}

#[tokio::test]
async fn cancellation_cleans_previously_read_upload() {
  tokio::time::timeout(Duration::from_secs(2), cancel_after_upload())
    .await
    .unwrap();
}

#[cfg(feature = "compio")]
#[compio::test]
async fn compio_cancellation_cleans_previously_read_upload() {
  compio::time::timeout(Duration::from_secs(2), cancel_after_upload())
    .await
    .unwrap();
}

#[tokio::test]
async fn file_reader_timeout_cleans_its_upload() {
  timed_out_reader_cleans_upload().await;
}

#[cfg(feature = "compio")]
#[compio::test]
async fn compio_file_reader_timeout_cleans_its_upload() {
  timed_out_reader_cleans_upload().await;
}

#[tokio::test]
async fn later_field_error_cleans_previously_read_upload() {
  let mut request = request(format!(
    "{}--probe\r\nContent-Disposition: form-data; name=\"extra\"\r\n\r\ntext\r\n--probe--\r\n",
    body_prefix()
  ));
  request
    .extensions_mut()
    .insert(MultipartConfig::new().max_parts(1));
  let result =
    TakoTypedMultipart::<serde_json::Value, ObservedFile>::from_request(&mut request).await;
  assert!(matches!(result, Err(TypedMultipartError::TooManyParts)));
  assert!(!observed_path().exists());
}

#[tokio::test]
async fn custom_wrapper_transfers_cleanup() {
  #[derive(Deserialize)]
  struct Form {
    file: UploadedFile,
  }

  let mut request = request(format!("{}--probe--\r\n", body_prefix()));
  let data = TakoTypedMultipart::<Form, ObservedFile>::from_request(&mut request)
    .await
    .unwrap()
    .data;
  let path = observed_path();
  assert_eq!(data.file.path, path);
  assert_eq!(std::fs::read(&path).unwrap(), b"hello");
  drop(data);
  assert!(!path.exists());
}

#[tokio::test]
async fn ignored_field_cleans_its_upload() {
  let mut request = request(format!("{}--probe--\r\n", body_prefix()));
  let data = TakoTypedMultipart::<serde_json::Value, ObservedFile>::from_request(&mut request)
    .await
    .unwrap()
    .data;
  assert_eq!(data["file"]["size"], 5);
  assert!(!observed_path().exists());
}
