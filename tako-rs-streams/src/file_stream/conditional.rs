//! Conditional request evaluation (RFC 9110 §13 preconditions).

use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use http::HeaderMap;
use http::StatusCode;
use tako_rs_core::body::TakoBody;
use tako_rs_core::types::Response;

use super::date::format_http_date;
use super::date::parse_http_date;

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

/// `ETag` comparison helper.
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
