use http::HeaderMap;
use http::header;
use tako_rs_core::body::TakoBody;
use tako_rs_core::types::Response;

use super::conditional::etag_match;

pub(crate) enum RequestedRange {
  Full,
  Partial { start: u64, length: u64 },
  Unsatisfiable,
}

pub(crate) fn requested_range(
  headers: &HeaderMap,
  size: u64,
  etag: Option<&str>,
) -> RequestedRange {
  use RequestedRange::Full;
  use RequestedRange::Partial;
  use RequestedRange::Unsatisfiable;
  if let Some(value) = headers.get(header::IF_RANGE) {
    // File timestamps alone cannot establish the strong validation If-Range requires.
    let matched = value
      .to_str()
      .ok()
      .zip(etag)
      .is_some_and(|(value, etag)| etag_match(value, etag, true));
    if !matched {
      return Full;
    }
  }
  let mut values = headers.get_all(header::RANGE).iter();
  let Some(value) = values.next().and_then(|v| v.to_str().ok()) else {
    return Full;
  };
  if values.next().is_some() {
    return Full;
  }
  let Some((unit, range)) = value.split_once('=') else {
    return Full;
  };
  if !unit.eq_ignore_ascii_case("bytes") || range.contains(',') {
    return Full;
  }
  let Some((start, end)) = range.trim().split_once('-') else {
    return Full;
  };
  let number = |value: &str| {
    if value.is_empty() || !value.bytes().all(|c| c.is_ascii_digit()) {
      None
    } else {
      value.parse::<u64>().ok()
    }
  };
  if size == 0 {
    return Full;
  }
  if start.is_empty() {
    let Some(suffix) = number(end) else {
      return Full;
    };
    if suffix == 0 {
      return Unsatisfiable;
    }
    let length = suffix.min(size);
    return Partial {
      start: size - length,
      length,
    };
  }
  let Some(start) = number(start) else {
    return Full;
  };
  let end = if end.is_empty() {
    size - 1
  } else {
    let Some(end) = number(end) else {
      return Full;
    };
    if end < start {
      return Full;
    }
    end.min(size - 1)
  };
  if start >= size {
    return Unsatisfiable;
  }
  Partial {
    start,
    length: end - start + 1,
  }
}

pub(crate) fn unsatisfiable(size: u64) -> Response {
  http::Response::builder()
    .status(http::StatusCode::RANGE_NOT_SATISFIABLE)
    .header(header::CONTENT_RANGE, format!("bytes */{size}"))
    .body(TakoBody::empty())
    .expect("valid file range response")
}
