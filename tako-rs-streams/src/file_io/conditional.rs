//! Preconditions for an existing selected representation.

use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use http::HeaderMap;
use http::Method;
use http::StatusCode;
use http::header;
use tako_rs_core::body::TakoBody;
use tako_rs_core::types::Response;

use super::date::format_http_date;
use super::date::parse_http_date;

/// Evaluate GET/HEAD preconditions for an existing representation.
///
/// Returns 412 for a failed write validator, 304 for a cache hit, or `None`
/// to proceed. `ETag` values must be quoted, optionally prefixed with `W/`.
/// For other methods, use [`evaluate_conditional_for_method`].
pub fn evaluate_conditional(
  headers: &HeaderMap,
  etag: Option<&str>,
  modified: Option<SystemTime>,
) -> Option<Response> {
  evaluate_conditional_for_method(&Method::GET, headers, etag, modified)
}

/// Evaluate preconditions for an existing representation using the request method.
///
/// A matching `If-None-Match` returns 304 for GET/HEAD and 412 otherwise.
pub fn evaluate_conditional_for_method(
  method: &Method,
  headers: &HeaderMap,
  etag: Option<&str>,
  modified: Option<SystemTime>,
) -> Option<Response> {
  let safe = method == Method::GET || method == Method::HEAD;
  let modified_secs = modified
    .and_then(|ts| ts.duration_since(UNIX_EPOCH).ok())
    .map(|d| d.as_secs());
  if headers.contains_key(header::IF_MATCH) {
    if !matches(headers, header::IF_MATCH, etag, true) {
      return Some(empty(StatusCode::PRECONDITION_FAILED));
    }
  } else if let (Some(since), Some(modified)) =
    (date(headers, header::IF_UNMODIFIED_SINCE), modified_secs)
    && modified > since
  {
    return Some(empty(StatusCode::PRECONDITION_FAILED));
  }
  let unchanged = if headers.contains_key(header::IF_NONE_MATCH) {
    matches(headers, header::IF_NONE_MATCH, etag, false)
  } else {
    safe
      && date(headers, header::IF_MODIFIED_SINCE)
        .zip(modified_secs)
        .is_some_and(|(since, modified)| modified <= since)
  };
  if !unchanged {
    return None;
  }
  if !safe {
    return Some(empty(StatusCode::PRECONDITION_FAILED));
  }
  let mut response = empty(StatusCode::NOT_MODIFIED);
  if let Some(etag) = etag.and_then(|s| s.parse().ok()) {
    response.headers_mut().insert(header::ETAG, etag);
  }
  if let Some(modified) = modified_secs {
    response.headers_mut().insert(
      header::LAST_MODIFIED,
      format_http_date(modified).parse().unwrap(),
    );
  }
  Some(response)
}

fn matches(
  headers: &HeaderMap,
  name: header::HeaderName,
  etag: Option<&str>,
  strong: bool,
) -> bool {
  headers
    .get_all(name)
    .iter()
    .filter_map(|v| v.to_str().ok())
    .any(|value| value.trim() == "*" || etag.is_some_and(|etag| etag_match(value, etag, strong)))
}

fn date(headers: &HeaderMap, name: header::HeaderName) -> Option<u64> {
  parse_http_date(headers.get(name)?.to_str().ok()?)
}

pub(crate) fn etag_match(mut list: &str, value: &str, strong: bool) -> bool {
  if strong && value.starts_with("W/") {
    return false;
  }
  let value = value.strip_prefix("W/").unwrap_or(value);
  while !list.is_empty() {
    list = list.trim_start();
    let weak = list.starts_with("W/");
    let raw = list.strip_prefix("W/").unwrap_or(list);
    let Some(rest) = raw.strip_prefix('"') else {
      return false;
    };
    let Some(end) = rest.find('"') else {
      return false;
    };
    let tag = &raw[..end + 2];
    let remaining = raw[end + 2..].trim_start();
    if !(remaining.is_empty() || remaining.starts_with(',')) {
      return false;
    }
    if !(strong && weak) && tag == value {
      return true;
    }
    list = remaining.strip_prefix(',').unwrap_or(remaining);
  }
  false
}

fn empty(status: StatusCode) -> Response {
  let mut response = Response::new(TakoBody::empty());
  *response.status_mut() = status;
  response
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn quoted_etags_support_weak_comparison_and_commas() {
    let mut headers = HeaderMap::new();
    headers.insert(
      header::IF_NONE_MATCH,
      "W/\"other\", \"a,b\"".parse().unwrap(),
    );
    assert_eq!(
      evaluate_conditional(&headers, Some("W/\"a,b\""), None)
        .unwrap()
        .status(),
      StatusCode::NOT_MODIFIED
    );
    assert_eq!(
      evaluate_conditional_for_method(&Method::PUT, &headers, Some("\"a,b\""), None)
        .unwrap()
        .status(),
      StatusCode::PRECONDITION_FAILED
    );
    headers.insert(header::IF_NONE_MATCH, "*".parse().unwrap());
    assert_eq!(
      evaluate_conditional(&headers, None, None).unwrap().status(),
      StatusCode::NOT_MODIFIED
    );
  }

  #[test]
  fn if_match_uses_strong_comparison_and_overrides_the_date() {
    let mut headers = HeaderMap::new();
    headers.insert(header::IF_MATCH, "\"same\"".parse().unwrap());
    headers.insert(
      header::IF_UNMODIFIED_SINCE,
      "Thu, 01 Jan 1970 00:00:00 GMT".parse().unwrap(),
    );
    let modified = UNIX_EPOCH + std::time::Duration::from_secs(100);
    assert!(evaluate_conditional(&headers, Some("\"same\""), Some(modified)).is_none());
    assert_eq!(
      evaluate_conditional(&headers, Some("W/\"same\""), Some(modified))
        .unwrap()
        .status(),
      StatusCode::PRECONDITION_FAILED
    );
    headers.insert(header::IF_MATCH, "W/\"same\"".parse().unwrap());
    assert_eq!(
      evaluate_conditional(&headers, Some("\"same\""), Some(modified))
        .unwrap()
        .status(),
      StatusCode::PRECONDITION_FAILED
    );
  }
}
