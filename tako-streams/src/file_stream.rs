//! File streaming utilities for efficient HTTP file delivery.
//!
//! This module provides `FileStream` for streaming files over HTTP with support for
//! range requests, content-length headers, and proper MIME type detection. It enables
//! efficient delivery of large files without loading them entirely into memory, making
//! it suitable for serving media files, downloads, and other binary content.
//!
//! # Examples
//!
//! ```rust,ignore
//! use tako::file_stream::FileStream;
//! use tako::responder::Responder;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Stream a file from disk
//! let file_stream = FileStream::from_path("./assets/video.mp4").await?;
//! let response = file_stream.into_response();
//! # Ok(())
//! # }
//! ```

#![cfg_attr(docsrs, doc(cfg(feature = "file-stream")))]

#[cfg(not(feature = "compio"))]
use std::io::SeekFrom;
use std::path::Path;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Result;
use bytes::Bytes;
use futures_util::TryStream;
use futures_util::TryStreamExt;
use http::HeaderMap;
use http::StatusCode;
use http_body::Frame;
use sha1::Digest as _;
use sha1::Sha1;
use tako_core::body::TakoBody;
use tako_core::responder::Responder;
use tako_core::types::BoxError;
use tako_core::types::Response;
#[cfg(not(feature = "compio"))]
use tokio::fs::File;
#[cfg(not(feature = "compio"))]
use tokio::io::AsyncReadExt;
#[cfg(not(feature = "compio"))]
use tokio::io::AsyncSeekExt;
#[cfg(not(feature = "compio"))]
use tokio_util::io::ReaderStream;

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
  /// Optional pre-computed strong ETag value (without quotes).
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

  /// Attach a strong ETag (caller-provided digest, no quotes around the value).
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
      .map(|n| n.to_owned());

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
  pub fn into_range_response(self, start: u64, end: u64, total_size: u64) -> Response {
    let mut response = http::Response::builder()
      .status(http::StatusCode::PARTIAL_CONTENT)
      .header(
        http::header::CONTENT_TYPE,
        mime::APPLICATION_OCTET_STREAM.as_ref(),
      )
      .header(
        http::header::CONTENT_RANGE,
        format!("bytes {}-{}/{}", start, end, total_size),
      )
      .header(http::header::CONTENT_LENGTH, (end - start + 1).to_string());

    if let Some(ref name) = self.file_name {
      response = response.header(
        http::header::CONTENT_DISPOSITION,
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
        http::StatusCode::INTERNAL_SERVER_ERROR,
        format!("FileStream range error: {}", e),
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
  #[cfg(feature = "compio")]
  pub async fn try_range_response<P>(path: P, start: u64, mut end: u64) -> Result<Response>
  where
    P: AsRef<Path>,
  {
    let data = compio::fs::read(&path).await?;
    let total_size = data.len() as u64;

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
        format!("attachment; filename=\"{}\"", name),
      );
    }

    if let Some(ref etag) = self.etag {
      response = response.header(http::header::ETAG, format!("\"{}\"", etag));
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
        format!("FileStream error: {}", e),
      )
        .into_response()
    })
  }
}

/// Conditional GET evaluator.
///
/// Evaluates `If-None-Match` against `etag` and `If-Modified-Since` against
/// `last_modified`. Returns `Some(304 Not Modified response)` when the cache
/// hit is honored; `None` to proceed with the full response.
pub fn evaluate_conditional(
  request_headers: &HeaderMap,
  etag: Option<&str>,
  last_modified: Option<SystemTime>,
) -> Option<Response> {
  if let (Some(req), Some(etag)) = (request_headers.get(http::header::IF_NONE_MATCH), etag) {
    let req = req.to_str().unwrap_or("");
    if etag_match(req, etag) {
      return Some(not_modified(etag, last_modified));
    }
  }
  if let (Some(req), Some(ts)) = (
    request_headers.get(http::header::IF_MODIFIED_SINCE),
    last_modified,
  ) && let Ok(req) = req.to_str()
    && let Some(req_ts) = parse_http_date(req)
    && let Ok(file_ts) = ts.duration_since(UNIX_EPOCH)
    && file_ts.as_secs() <= req_ts
  {
    return Some(not_modified(etag.unwrap_or(""), Some(ts)));
  }
  None
}

fn not_modified(etag: &str, last_modified: Option<SystemTime>) -> Response {
  let mut builder = http::Response::builder().status(StatusCode::NOT_MODIFIED);
  if !etag.is_empty() {
    builder = builder.header(http::header::ETAG, format!("\"{}\"", etag));
  }
  if let Some(ts) = last_modified
    && let Ok(s) = ts.duration_since(UNIX_EPOCH)
  {
    builder = builder.header(http::header::LAST_MODIFIED, format_http_date(s.as_secs()));
  }
  builder.body(TakoBody::empty()).expect("valid 304 response")
}

fn etag_match(header: &str, value: &str) -> bool {
  if header.trim() == "*" {
    return true;
  }
  for raw in header.split(',') {
    let raw = raw.trim();
    let raw = raw.strip_prefix("W/").unwrap_or(raw);
    let raw = raw.trim_matches('"');
    if raw == value {
      return true;
    }
  }
  false
}

/// IMF-fixdate (RFC 7231) formatter, sufficient for `Last-Modified` and `Date`.
fn format_http_date(unix_secs: u64) -> String {
  let days = unix_secs / 86400;
  let secs_of_day = unix_secs % 86400;
  let h = secs_of_day / 3600;
  let m = (secs_of_day % 3600) / 60;
  let s = secs_of_day % 60;

  let dow_idx = (days + 4) % 7;
  let dow_name = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][dow_idx as usize];

  let (year, month, day) = epoch_days_to_ymd(days as i64);
  let mon_name = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
  ][(month - 1) as usize];

  format!(
    "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
    dow_name, day, mon_name, year, h, m, s
  )
}

/// Reverse of `format_http_date` accepting `IMF-fixdate` only. Returns the
/// epoch second value or `None` for unsupported formats.
fn parse_http_date(header: &str) -> Option<u64> {
  let header = header.trim();
  // "Sun, 06 Nov 1994 08:49:37 GMT"
  let bytes = header.as_bytes();
  if bytes.len() < 29 {
    return None;
  }
  let day: u64 = std::str::from_utf8(&bytes[5..7]).ok()?.parse().ok()?;
  let month_str = std::str::from_utf8(&bytes[8..11]).ok()?;
  let month = match month_str {
    "Jan" => 1,
    "Feb" => 2,
    "Mar" => 3,
    "Apr" => 4,
    "May" => 5,
    "Jun" => 6,
    "Jul" => 7,
    "Aug" => 8,
    "Sep" => 9,
    "Oct" => 10,
    "Nov" => 11,
    "Dec" => 12,
    _ => return None,
  };
  let year: i64 = std::str::from_utf8(&bytes[12..16]).ok()?.parse().ok()?;
  let h: u64 = std::str::from_utf8(&bytes[17..19]).ok()?.parse().ok()?;
  let m: u64 = std::str::from_utf8(&bytes[20..22]).ok()?.parse().ok()?;
  let s: u64 = std::str::from_utf8(&bytes[23..25]).ok()?.parse().ok()?;

  let days = ymd_to_epoch_days(year, month, day as i64);
  Some((days as u64) * 86400 + h * 3600 + m * 60 + s)
}

fn is_leap(y: i64) -> bool {
  (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn ymd_to_epoch_days(year: i64, month: i64, day: i64) -> i64 {
  let mut days: i64 = 0;
  let cmp = |a: i64, b: i64| (a > b) as i64;

  if year >= 1970 {
    for y in 1970..year {
      days += if is_leap(y) { 366 } else { 365 };
    }
  } else {
    for y in year..1970 {
      days -= if is_leap(y) { 366 } else { 365 };
    }
  }
  let mdays = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  for m in 1..month {
    days += mdays[(m - 1) as usize];
    if m == 2 && is_leap(year) {
      days += 1;
    }
  }
  days += day - 1;
  let _ = cmp;
  days
}

fn epoch_days_to_ymd(days: i64) -> (i64, i64, i64) {
  // Civil from days since 1970-01-01 — Howard Hinnant algorithm.
  let z = days + 719468;
  let era = if z >= 0 { z } else { z - 146096 } / 146097;
  let doe = (z - era * 146097) as i64;
  let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
  let y = yoe + era * 400;
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  let mp = (5 * doy + 2) / 153;
  let d = doy - (153 * mp + 2) / 5 + 1;
  let m = if mp < 10 { mp + 3 } else { mp - 9 };
  let y = if m <= 2 { y + 1 } else { y };
  (y, m, d)
}

/// Helper that hashes a file path's canonicalized name + size + mtime into a
/// SHA-1 strong ETag. Cheap (no body read) but stable across restarts.
pub fn weak_etag_from_metadata(size: u64, mtime: SystemTime) -> String {
  let mtime_secs = mtime
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0);
  let mut hasher = Sha1::new();
  hasher.update(size.to_le_bytes());
  hasher.update(mtime_secs.to_le_bytes());
  let digest = hasher.finalize();
  let mut hex = String::with_capacity(40);
  for b in digest {
    hex.push_str(&format!("{:02x}", b));
  }
  hex
}
