//! File streaming utilities for efficient HTTP file delivery.
//!
//! This module provides `FileStream` for streaming files over HTTP with support for
//! range requests, content-length headers, and proper MIME type detection. It enables
//! efficient delivery of large files without loading them entirely into memory, making
//! it suitable for serving media files, downloads, and other binary content.
//!
//! # Examples
//!
//! ```rust
//! use tako::file_stream::FileStream;
//! use tako::responder::Responder;
//! use tokio_util::io::ReaderStream;
//! use tokio::fs::File;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Stream a file from disk
//! let file_stream = FileStream::from_path("./assets/video.mp4").await?;
//! let response = file_stream.into_response();
//!
//! // Create a custom stream with metadata
//! let file = File::open("./data.bin").await?;
//! let reader_stream = ReaderStream::new(file);
//! let custom_stream = FileStream::new(
//!     reader_stream,
//!     Some("download.bin".to_string()),
//!     Some(1024),
//! );
//! let response = custom_stream.into_response();
//! # Ok(())
//! # }
//! ```

use std::{io::SeekFrom, path::Path};

use anyhow::Result;
use bytes::Bytes;
use futures_util::{TryStream, TryStreamExt};
use hyper::{StatusCode, body::Frame};
use tokio::{
  fs::File,
  io::{AsyncReadExt, AsyncSeekExt},
};
use tokio_util::io::ReaderStream;

use crate::{
  body::TakoBody,
  responder::Responder,
  types::{BoxError, Response},
};

/// HTTP file stream with metadata support for efficient file delivery.
///
/// `FileStream` wraps any stream that produces bytes and associates it with optional
/// metadata like filename and content size. This enables proper HTTP headers to be
/// set for file downloads, including Content-Disposition for filename suggestions
/// and Content-Length for known file sizes. The implementation supports both
/// regular responses and HTTP range requests for partial content delivery.
///
/// # Examples
///
/// ```rust
/// use tako::file_stream::FileStream;
/// use tokio_util::io::ReaderStream;
/// use tokio::fs::File;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // From file path (recommended)
/// let stream = FileStream::from_path("./video.mp4").await?;
///
/// // From custom stream
/// let file = File::open("./data.txt").await?;
/// let reader = ReaderStream::new(file);
/// let stream = FileStream::new(reader, Some("data.txt".to_string()), Some(2048));
/// # Ok(())
/// # }
/// ```
pub struct FileStream<S> {
  /// The underlying byte stream
  pub stream: S,
  /// Optional filename for Content-Disposition header
  pub file_name: Option<String>,
  /// Optional content size for Content-Length header
  pub content_size: Option<u64>,
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
    }
  }

  /// Creates a file stream from a file system path with automatic metadata detection.
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
    })
  }

  /// Creates an HTTP 206 Partial Content response for range requests.
  pub fn into_range_response(self, start: u64, end: u64, total_size: u64) -> Response {
    let mut response = hyper::Response::builder()
      .status(hyper::StatusCode::PARTIAL_CONTENT)
      .header(
        hyper::header::CONTENT_TYPE,
        mime::APPLICATION_OCTET_STREAM.as_ref(),
      )
      .header(
        hyper::header::CONTENT_RANGE,
        format!("bytes {}-{}/{}", start, end, total_size),
      )
      .header(hyper::header::CONTENT_LENGTH, (end - start + 1).to_string());

    if let Some(ref name) = self.file_name {
      response = response.header(
        hyper::header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{}\"", name),
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
        hyper::StatusCode::INTERNAL_SERVER_ERROR,
        format!("FileStream range error: {}", e),
      )
        .into_response()
    })
  }

  /// Try to create a range response for a file stream.
  pub async fn try_range_response<P>(path: P, start: u64, mut end: u64) -> Result<Response>
  where
    P: AsRef<Path>,
  {
    let mut file = File::open(path).await?;
    let meta = file.metadata().await?;
    let total_size = meta.len();

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
}

impl<S> Responder for FileStream<S>
where
  S: TryStream + Send + 'static,
  S::Ok: Into<Bytes>,
  S::Error: Into<BoxError>,
{
  /// Converts the file stream into an HTTP response with appropriate headers.
  fn into_response(self) -> Response {
    let mut response = hyper::Response::builder()
      .status(hyper::StatusCode::OK)
      .header(
        hyper::header::CONTENT_TYPE,
        mime::APPLICATION_OCTET_STREAM.as_ref(),
      );

    if let Some(size) = self.content_size {
      response = response.header(hyper::header::CONTENT_LENGTH, size.to_string());
    }

    if let Some(ref name) = self.file_name {
      response = response.header(
        hyper::header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{}\"", name),
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
        hyper::StatusCode::INTERNAL_SERVER_ERROR,
        format!("FileStream error: {}", e),
      )
        .into_response()
    })
  }
}
