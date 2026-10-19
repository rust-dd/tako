use std::path::Path;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Result;
use bytes::Bytes;
use futures_util::TryStream;
use futures_util::TryStreamExt;
use http::StatusCode;
use http::header;
use http_body::Frame;
use tako_rs_core::body::TakoBody;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::BoxError;
use tako_rs_core::types::Response;

use crate::file_io::FileByteStream;
use crate::file_io::OpenFile;
use crate::file_io::date::format_http_date;
use crate::file_io::etag::weak_etag_from_metadata;
use crate::file_io::range::unsatisfiable;

/// A byte stream response with optional download metadata. With `compio`, poll and drop a
/// disk-backed stream on the thread that opened the file.
#[doc(alias = "file_stream")]
pub struct FileStream<S = FileByteStream> {
  pub stream: S,
  /// Sent as `Content-Disposition: attachment`.
  pub file_name: Option<String>,
  /// Sent as `Content-Length`.
  pub content_size: Option<u64>,
  /// Quoted, optionally prefixed with `W/`.
  pub etag: Option<String>,
  pub last_modified: Option<SystemTime>,
  /// Defaults to `application/octet-stream`.
  pub content_type: Option<String>,
}

impl<S> FileStream<S>
where
  S: TryStream + Send + 'static,
  S::Ok: Into<Bytes>,
  S::Error: Into<BoxError>,
{
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

  /// `etag` must be quoted, optionally prefixed with `W/`.
  pub fn with_etag(mut self, etag: impl Into<String>) -> Self {
    self.etag = Some(etag.into());
    self
  }

  pub fn with_last_modified(mut self, timestamp: SystemTime) -> Self {
    self.last_modified = Some(timestamp);
    self
  }

  pub fn with_content_type(mut self, content_type: impl Into<String>) -> Self {
    self.content_type = Some(content_type.into());
    self
  }

  /// A 206 for a stream already positioned at `start` and limited to the range; bounds outside
  /// `start <= end < total_size` yield 416.
  pub fn into_range_response(self, start: u64, end: u64, total_size: u64) -> Response {
    if start > end || end >= total_size {
      return unsatisfiable(total_size);
    }
    self.response(Some((start, end, total_size)))
  }

  fn response(self, range: Option<(u64, u64, u64)>) -> Response {
    let mut builder = http::Response::builder()
      .status(if range.is_some() {
        StatusCode::PARTIAL_CONTENT
      } else {
        StatusCode::OK
      })
      .header(
        header::CONTENT_TYPE,
        self
          .content_type
          .as_deref()
          .unwrap_or("application/octet-stream"),
      );
    if let Some((start, end, size)) = range {
      builder = builder
        .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{size}"))
        .header(header::CONTENT_LENGTH, end - start + 1);
    } else if let Some(size) = self.content_size {
      builder = builder.header(header::CONTENT_LENGTH, size);
    }
    if let Some(name) = self.file_name {
      let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
      builder = builder.header(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{escaped}\""),
      );
    }
    if let Some(etag) = self.etag {
      builder = builder.header(header::ETAG, etag);
    }
    if let Some(modified) = self
      .last_modified
      .and_then(|ts| ts.duration_since(UNIX_EPOCH).ok())
    {
      builder = builder.header(header::LAST_MODIFIED, format_http_date(modified.as_secs()));
    }
    let body = TakoBody::from_try_stream(
      self
        .stream
        .map_ok(|chunk| Frame::data(chunk.into()))
        .map_err(Into::into),
    );
    builder.body(body).unwrap_or_else(|error| {
      tracing::error!(%error, "invalid file response metadata");
      StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })
  }
}

impl FileStream<FileByteStream> {
  /// Streams a regular file with a weak `ETag` and a content type guessed from its extension.
  pub async fn from_path(path: impl AsRef<Path>) -> Result<Self> {
    let file = OpenFile::open(path.as_ref()).await?;
    let size = file.size;
    let modified = file.modified;
    Ok(Self {
      stream: file.into_stream(0, size).await?,
      file_name: path
        .as_ref()
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned),
      content_size: Some(size),
      etag: modified.map(|ts| weak_etag_from_metadata(size, ts)),
      last_modified: modified,
      content_type: Some(
        mime_guess::from_path(path)
          .first_or_octet_stream()
          .to_string(),
      ),
    })
  }

  /// Streams the inclusive range `start..=end`, where an `end` past EOF, such as `u64::MAX`,
  /// means through EOF; an unsatisfiable range yields 416.
  pub async fn try_range_response(
    path: impl AsRef<Path>,
    start: u64,
    end: u64,
  ) -> Result<Response> {
    let file = OpenFile::open(path.as_ref()).await?;
    let size = file.size;
    if size == 0 || start >= size || start > end {
      return Ok(unsatisfiable(size));
    }
    let end = end.min(size - 1);
    let modified = file.modified;
    let stream = Self {
      stream: file.into_stream(start, end - start + 1).await?,
      file_name: None,
      content_size: None,
      etag: modified.map(|ts| weak_etag_from_metadata(size, ts)),
      last_modified: modified,
      content_type: Some(
        mime_guess::from_path(path)
          .first_or_octet_stream()
          .to_string(),
      ),
    };
    Ok(stream.into_range_response(start, end, size))
  }
}

impl<S> Responder for FileStream<S>
where
  S: TryStream + Send + 'static,
  S::Ok: Into<Bytes>,
  S::Error: Into<BoxError>,
{
  fn into_response(self) -> Response {
    self.response(None)
  }
}
