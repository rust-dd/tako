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
//! // `Stream` a file from disk
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
  /// (`W/"abc"`) for a weak one. Use [`weak_etag_from_metadata`] to derive a
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
        .header(
          http::header::CONTENT_RANGE,
          format!("bytes */{total_size}"),
        )
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

/// Conditional GET / PUT evaluator (RFC 9110 §13.1).
///
/// Returns:
/// - `Some(412 Precondition Failed)` when `If-Match` or `If-Unmodified-Since`
///   would not be satisfied — caller must abort writes / state-changes.
/// - `Some(304 Not Modified)` for safe-method cache hits.
/// - `None` to proceed with the full response.
pub fn evaluate_conditional(
  request_headers: &HeaderMap,
  etag: Option<&str>,
  last_modified: Option<SystemTime>,
) -> Option<Response> {
  // Step 1 (RFC 9110 §13.2.2): `If-Match` — if any listed validator matches
  // the current ETag, proceed; otherwise 412. STR-3: must use the *strong*
  // comparison function (§13.1.1 / §8.8.3.2). Tako's `weak_etag_from_metadata`
  // emits `W/"..."` validators, so the previous weak-stripping comparison
  // let weak request-side entries succeed against a weak server-side ETag —
  // a spec violation that effectively turned `If-Match` into the weaker
  // sibling of `If-None-Match`.
  if let Some(req) = request_headers.get(http::header::IF_MATCH) {
    let req = req.to_str().unwrap_or("");
    let satisfied = match etag {
      Some(e) => etag_match(req, e, /* strong_only */ true),
      None => req.trim() == "*",
    };
    if !satisfied {
      return Some(precondition_failed());
    }
  }

  // Step 2: `If-Unmodified-Since` — caller-provided lower bound on the
  // file's mtime; if the file is newer, 412.
  if let (Some(req), Some(ts)) = (
    request_headers.get(http::header::IF_UNMODIFIED_SINCE),
    last_modified,
  ) && let Ok(req) = req.to_str()
    && let Some(req_ts) = parse_http_date(req)
    && let Ok(file_ts) = ts.duration_since(UNIX_EPOCH)
    && file_ts.as_secs() > req_ts
  {
    return Some(precondition_failed());
  }

  // Step 3: `If-None-Match` — same-validator → 304. Per RFC 9110 §13.1.2
  // this uses *weak* comparison: a `W/"abc"` request entry matches a
  // strong or weak server-side `"abc"` / `W/"abc"`. Caching gates are the
  // canonical use-case for weak comparison.
  if let (Some(req), Some(etag)) = (request_headers.get(http::header::IF_NONE_MATCH), etag) {
    let req = req.to_str().unwrap_or("");
    if etag_match(req, etag, /* strong_only */ false) {
      return Some(not_modified(etag, last_modified));
    }
  }

  // Step 4: `If-Modified-Since` — coarse mtime check.
  //
  // STR-2: RFC 9110 §13.1.3 mandates that `If-Modified-Since` MUST be
  // ignored if `If-None-Match` is present (either matched or unmatched in
  // step 3). The previous code skipped this guard and let a stale mtime
  // override a deliberate `If-None-Match` non-match — clients could be
  // served `304` for stale-but-still-recent files even though the ETag
  // said the body changed.
  if request_headers.get(http::header::IF_NONE_MATCH).is_none()
    && let (Some(req), Some(ts)) = (
      request_headers.get(http::header::IF_MODIFIED_SINCE),
      last_modified,
    )
    && let Ok(req) = req.to_str()
    && let Some(req_ts) = parse_http_date(req)
    && let Ok(file_ts) = ts.duration_since(UNIX_EPOCH)
    && file_ts.as_secs() <= req_ts
  {
    return Some(not_modified(etag.unwrap_or(""), Some(ts)));
  }
  None
}

fn precondition_failed() -> Response {
  http::Response::builder()
    .status(StatusCode::PRECONDITION_FAILED)
    .body(TakoBody::empty())
    .expect("valid 412 response")
}

fn not_modified(etag: &str, last_modified: Option<SystemTime>) -> Response {
  let mut builder = http::Response::builder().status(StatusCode::NOT_MODIFIED);
  if !etag.is_empty() {
    builder = builder.header(http::header::ETAG, etag);
  }
  if let Some(ts) = last_modified
    && let Ok(s) = ts.duration_since(UNIX_EPOCH)
  {
    builder = builder.header(http::header::LAST_MODIFIED, format_http_date(s.as_secs()));
  }
  builder.body(TakoBody::empty()).expect("valid 304 response")
}

/// ETag comparison helper.
///
/// `strong_only = true` matches RFC 9110 §13.1.1 / §8.8.3.2 strong
/// comparison: weak (`W/`-prefixed) entries in EITHER the request header or
/// the server value are rejected — required for `If-Match` and any other
/// precondition that mutates state on success. `strong_only = false`
/// performs weak comparison (strips the `W/` prefix from request entries
/// before equality), used by `If-None-Match` per RFC 9110 §13.1.2.
fn etag_match(header: &str, value: &str, strong_only: bool) -> bool {
  if header.trim() == "*" {
    return true;
  }
  if strong_only && value.starts_with("W/") {
    // Strong comparison: weak server-side ETag never matches.
    return false;
  }
  for raw in header.split(',') {
    let raw = raw.trim();
    if strong_only && raw.starts_with("W/") {
      // Strong comparison: weak request-side entry is silently skipped.
      continue;
    }
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

  format!("{dow_name}, {day:02} {mon_name} {year:04} {h:02}:{m:02}:{s:02} GMT")
}

/// Parse an HTTP-date header value into Unix epoch seconds.
///
/// Delegates to the `httpdate` crate which accepts every format RFC 9110
/// §5.6.7 lists: IMF-fixdate (`Sun, 06 Nov 1994 08:49:37 GMT`), RFC 850
/// (`Sunday, 06-Nov-94 08:49:37 GMT`), and asctime (`Sun Nov 6 08:49:37 1994`).
/// The previous hand-rolled IMF-fixdate-only parser rejected legitimate
/// clients (Java/.NET defaults still emit RFC 850 in places) and forced the
/// server to ship full bodies on `If-Modified-Since` despite a fresh cache.
fn parse_http_date(header: &str) -> Option<u64> {
  let st = httpdate::parse_http_date(header.trim()).ok()?;
  st.duration_since(std::time::UNIX_EPOCH)
    .ok()
    .map(|d| d.as_secs())
}

fn epoch_days_to_ymd(days: i64) -> (i64, i64, i64) {
  // Civil from days since 1970-01-01 — Howard Hinnant algorithm.
  let z = days + 719_468;
  let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
  let doe = z - era * 146_097;
  let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
  let y = yoe + era * 400;
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  let mp = (5 * doy + 2) / 153;
  let d = doy - (153 * mp + 2) / 5 + 1;
  let m = if mp < 10 { mp + 3 } else { mp - 9 };
  let y = if m <= 2 { y + 1 } else { y };
  (y, m, d)
}

/// Helper that hashes (size + mtime) into a **weak** `ETag` (`W/"…"`).
///
/// SHA-1 over coarse metadata cannot prove byte-for-byte equivalence — two
/// files written within the same wall-clock second with the same size will
/// hash to the same digest. Returning the value pre-wrapped in `W/"…"` keeps
/// downstream callers honest about that limitation: clients (and caches)
/// won't assume strong validation semantics. Callers should pass the value
/// straight to `Response.header(ETAG, …)` without re-quoting.
pub fn weak_etag_from_metadata(size: u64, mtime: SystemTime) -> String {
  let mtime_secs = mtime.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
  let mut hasher = Sha1::new();
  hasher.update(size.to_le_bytes());
  hasher.update(mtime_secs.to_le_bytes());
  let digest = hasher.finalize();
  let mut out = String::with_capacity(44);
  out.push_str("W/\"");
  for b in digest {
    out.push_str(&format!("{b:02x}"));
  }
  out.push('"');
  out
}
