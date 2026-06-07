//! The `FileStream` type, its constructors, and HTTP response conversions.

#[cfg(not(feature = "compio"))]
use std::io::SeekFrom;
use std::path::Path;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Result;
use bytes::Bytes;
use futures_util::TryStream;
use futures_util::TryStreamExt;
use http::StatusCode;
use http_body::Frame;
use tako_rs_core::body::TakoBody;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::BoxError;
use tako_rs_core::types::Response;
#[cfg(not(feature = "compio"))]
use tokio::fs::File;
#[cfg(not(feature = "compio"))]
use tokio::io::AsyncReadExt;
#[cfg(not(feature = "compio"))]
use tokio::io::AsyncSeekExt;
#[cfg(not(feature = "compio"))]
use tokio_util::io::ReaderStream;

use super::date::format_http_date;

/// HTTP file stream with metadata support for efficient file delivery.
///
/// `FileStream` wraps any stream that produces bytes and associates it with optional
/// metadata like filename and content size. This enables proper HTTP headers to be
/// set for file downloads, including Content-Disposition for filename suggestions
/// and Content-Length for known file sizes. The implementation supports both
/// regular responses and HTTP range requests for partial content delivery.
#[doc(alias = "file_stream")]
#[doc(alias = "stream")]
pub struct FileStream<S> {
  /// The underlying byte stream
  pub stream: S,
  /// Optional filename for Content-Disposition header
  pub file_name: Option<String>,
  /// Optional content size for Content-Length header
  pub content_size: Option<u64>,
  /// Optional pre-computed strong `ETag` value (without quotes).
  pub etag: Option<String>,
  /// Optional last-modified timestamp.
  pub last_modified: Option<SystemTime>,
  /// Optional content-type override (defaults to `application/octet-stream`).
  pub content_type: Option<String>,
}

impl<S> FileStream<S>
where
  S: TryStream + Send + 'static,
  S::Ok: Into<Bytes>,
  S::Error: Into<BoxError>,
{
  /// Creates a new file stream with the provided metadata.
  pub fn new(stream: S, file_name: Option<String>, content_size: Option<u64>) -> Self {
    Self {
      stream,
      file_name,
      content_size,
      etag: None,
      last_modified: None,
      content_type: None,
    }
  }

  /// Attach an `ETag` validator. The value must be fully formed per RFC 9110
  /// §8.8.3 — i.e. quoted (`"abc"`) for a strong validator or weak-prefixed
  /// (`W/"abc"`) for a weak one. Use [`weak_etag_from_metadata`](super::weak_etag_from_metadata) to derive a
  /// weak validator from `(size, mtime)`.
  pub fn with_etag(mut self, etag: impl Into<String>) -> Self {
    self.etag = Some(etag.into());
    self
  }

  /// Attach a `Last-Modified` timestamp.
  pub fn with_last_modified(mut self, ts: SystemTime) -> Self {
    self.last_modified = Some(ts);
    self
  }

  /// Override the response `Content-Type` (defaults to `application/octet-stream`).
  pub fn with_content_type(mut self, ct: impl Into<String>) -> Self {
    self.content_type = Some(ct.into());
    self
  }

  /// Creates a file stream from a file system path with automatic metadata detection.
  #[cfg(not(feature = "compio"))]
  pub async fn from_path<P>(path: P) -> Result<FileStream<ReaderStream<File>>>
  where
    P: AsRef<Path>,
  {
    let file = File::open(&path).await?;
    let mut content_size = None;
    let mut file_name = None;

    if let Ok(metadata) = file.metadata().await {
      content_size = Some(metadata.len());
    }

    if let Some(os_name) = path.as_ref().file_name()
      && let Some(name) = os_name.to_str()
    {
      file_name = Some(name.to_owned());
    }

    Ok(FileStream {
      stream: ReaderStream::new(file),
      file_name,
      content_size,
      etag: None,
      last_modified: None,
      content_type: None,
    })
  }

  /// Creates a file stream from a file system path with automatic metadata detection (compio variant).
  ///
  /// ⚠️ **Memory-DoS warning:** the compio backend loads the entire file
  /// into a single `Bytes` allocation up front (`compio::fs::read`),
  /// unlike the tokio variant which streams chunks via `ReaderStream`.
  /// Do **not** expose this to untrusted requesters with arbitrary
  /// file paths; a multi-GB file (or many concurrent multi-GB requests)
  /// will allocate the full payload in RAM and can exhaust the process.
  /// For untrusted requests on the compio runtime, gate by
  /// max-file-size at the route level or write a custom positional-read
  /// streamer using `compio::fs::File::read_at` until tako-streams ships
  /// a streaming compio variant (tracked for 2.x).
  #[cfg(feature = "compio")]
  pub async fn from_path<P>(
    path: P,
  ) -> Result<
    FileStream<
      futures_util::stream::Once<futures_util::future::Ready<Result<Bytes, std::io::Error>>>,
    >,
  >
  where
    P: AsRef<Path>,
  {
    let data = compio::fs::read(&path).await?;
    let content_size = Some(data.len() as u64);
    let file_name = path
      .as_ref()
      .file_name()
      .and_then(|n| n.to_str())
      .map(std::borrow::ToOwned::to_owned);

    Ok(FileStream {
      stream: futures_util::stream::once(futures_util::future::ready(Ok(Bytes::from(data)))),
      file_name,
      content_size,
      etag: None,
      last_modified: None,
      content_type: None,
    })
  }

  /// Creates an HTTP 206 Partial Content response for range requests.
  ///
  /// Caller contract: `start <= end < total_size`. Violating the
  /// inequality used to panic on `end - start + 1`; we now return a
  /// `416 Range Not Satisfiable` response with a `Content-Range: bytes
  /// */{total_size}` header per RFC 9110 §15.5.17 instead, so a buggy
  /// caller produces a spec-conformant error rather than crashing the
  /// worker.
  pub fn into_range_response(self, start: u64, end: u64, total_size: u64) -> Response {
    if end < start || (total_size > 0 && end >= total_size) {
      return http::Response::builder()
        .status(http::StatusCode::RANGE_NOT_SATISFIABLE)
        .header(http::header::CONTENT_RANGE, format!("bytes */{total_size}"))
        .body(TakoBody::empty())
        .unwrap_or_else(|e| {
          (
            http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("FileStream range error: {e}"),
          )
            .into_response()
        });
    }
    let content_length = end.saturating_sub(start).saturating_add(1);
    let mut response = http::Response::builder()
      .status(http::StatusCode::PARTIAL_CONTENT)
      .header(
        http::header::CONTENT_TYPE,
        mime::APPLICATION_OCTET_STREAM.as_ref(),
      )
      .header(
        http::header::CONTENT_RANGE,
        format!("bytes {start}-{end}/{total_size}"),
      )
      .header(http::header::CONTENT_LENGTH, content_length.to_string());

    if let Some(ref name) = self.file_name {
      response = response.header(
        http::header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{name}\""),
      );
    }

    let body = TakoBody::from_try_stream(
      self
        .stream
        .map_ok(|chunk| Frame::data(Into::<Bytes>::into(chunk)))
        .map_err(Into::into),
    );

    response.body(body).unwrap_or_else(|e| {
      (
        http::StatusCode::INTERNAL_SERVER_ERROR,
        format!("FileStream range error: {e}"),
      )
        .into_response()
    })
  }

  /// Try to create a range response for a file stream.
  #[cfg(not(feature = "compio"))]
  pub async fn try_range_response<P>(path: P, start: u64, mut end: u64) -> Result<Response>
  where
    P: AsRef<Path>,
  {
    let mut file = File::open(path).await?;
    let meta = file.metadata().await?;
    let total_size = meta.len();

    // Empty file: any byte range is unsatisfiable. Guard before computing
    // `total_size - 1` so a zero-sized file does not underflow `u64`.
    if total_size == 0 {
      return Ok((StatusCode::RANGE_NOT_SATISFIABLE, "Range not satisfiable").into_response());
    }
    if end == 0 {
      end = total_size - 1;
    }

    if start > total_size || start > end || end >= total_size {
      return Ok((StatusCode::RANGE_NOT_SATISFIABLE, "Range not satisfiable").into_response());
    }

    file.seek(SeekFrom::Start(start)).await?;
    let stream = ReaderStream::new(file.take(end - start + 1));
    Ok(FileStream::new(stream, None, None).into_range_response(start, end, total_size))
  }

  /// Try to create a range response for a file stream (compio variant).
  ///
  /// ⚠️ Same memory-DoS caveat as [`FileStream::from_path`] (compio): the
  /// whole file is read into a single buffer before the requested range
  /// is sliced out, instead of doing a positional `read_at(start, len)`.
  /// Do not expose to untrusted requesters on arbitrary file paths;
  /// gate by file-size at the route level or switch to the tokio
  /// backend for streaming.
  #[cfg(feature = "compio")]
  pub async fn try_range_response<P>(path: P, start: u64, mut end: u64) -> Result<Response>
  where
    P: AsRef<Path>,
  {
    let data = compio::fs::read(&path).await?;
    let total_size = data.len() as u64;

    if total_size == 0 {
      return Ok((StatusCode::RANGE_NOT_SATISFIABLE, "Range not satisfiable").into_response());
    }
    if end == 0 {
      end = total_size - 1;
    }

    if start > total_size || start > end || end >= total_size {
      return Ok((StatusCode::RANGE_NOT_SATISFIABLE, "Range not satisfiable").into_response());
    }

    let slice = Bytes::from(data[(start as usize)..=(end as usize)].to_vec());
    let stream =
      futures_util::stream::once(futures_util::future::ready(Ok::<_, std::io::Error>(slice)));
    Ok(FileStream::new(stream, None, None).into_range_response(start, end, total_size))
  }
}

impl<S> Responder for FileStream<S>
where
  S: TryStream + Send + 'static,
  S::Ok: Into<Bytes>,
  S::Error: Into<BoxError>,
{
  /// Converts the file stream into an HTTP response with appropriate headers.
  fn into_response(self) -> Response {
    let ct = self
      .content_type
      .clone()
      .unwrap_or_else(|| mime::APPLICATION_OCTET_STREAM.as_ref().to_string());
    let mut response = http::Response::builder()
      .status(http::StatusCode::OK)
      .header(http::header::CONTENT_TYPE, ct);

    if let Some(size) = self.content_size {
      response = response.header(http::header::CONTENT_LENGTH, size.to_string());
    }

    if let Some(ref name) = self.file_name {
      response = response.header(
        http::header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{name}\""),
      );
    }

    if let Some(ref etag) = self.etag {
      response = response.header(http::header::ETAG, etag.as_str());
    }

    if let Some(ts) = self.last_modified
      && let Ok(s) = ts.duration_since(UNIX_EPOCH)
    {
      response = response.header(http::header::LAST_MODIFIED, format_http_date(s.as_secs()));
    }

    let body = TakoBody::from_try_stream(
      self
        .stream
        .map_ok(|chunk| Frame::data(Into::<Bytes>::into(chunk)))
        .map_err(Into::into),
    );

    response.body(body).unwrap_or_else(|e| {
      (
        http::StatusCode::INTERNAL_SERVER_ERROR,
        format!("FileStream error: {e}"),
      )
        .into_response()
    })
  }
}
