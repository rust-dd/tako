//! RFC 9110 `bytes=` [`Range`] header parsing; extract it as `Option<Range>`.

use http::HeaderMap;
use http::StatusCode;
use http::request::Parts;

use crate::extractors::Entries;
use crate::extractors::FromRequest;
use crate::extractors::FromRequestParts;
use crate::responder::Responder;
use crate::types::Request;

/// A single, parsed byte-range specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeSpec {
  /// `bytes=START-END` — inclusive on both ends.
  Inclusive { start: u64, end: u64 },
  /// `bytes=START-` — from `START` to the end of the representation.
  From { start: u64 },
  /// `bytes=-LENGTH` — last `LENGTH` bytes of the representation.
  Suffix { length: u64 },
}

impl RangeSpec {
  /// The inclusive `(start, end)` byte range within `total_size`, or `None` if unsatisfiable.
  pub fn resolve(self, total_size: u64) -> Option<(u64, u64)> {
    if total_size == 0 {
      return None;
    }
    let last = total_size - 1;
    match self {
      RangeSpec::Inclusive { start, end } => {
        if start > end || start > last {
          return None;
        }
        Some((start, end.min(last)))
      }
      RangeSpec::From { start } => {
        if start > last {
          return None;
        }
        Some((start, last))
      }
      RangeSpec::Suffix { length } => {
        if length == 0 {
          return None;
        }
        let length = length.min(total_size);
        Some((total_size - length, last))
      }
    }
  }
}

/// A parsed `Range` header.
#[derive(Debug, Clone)]
#[doc(alias = "range")]
pub struct Range {
  /// All ranges listed in the Range header, in client order. Always has at
  /// least one entry on a successful parse.
  pub specs: Vec<RangeSpec>,
}

impl Range {
  /// The first spec, for responders that serve a single range.
  #[must_use]
  pub fn single(&self) -> Option<RangeSpec> {
    self.specs.first().copied()
  }
}

/// Why the `Range` header was rejected; both cases answer 416.
#[derive(Debug)]
pub enum RangeError {
  /// Range header format is invalid (not `bytes=...` or contains a
  /// malformed range component).
  InvalidFormat,
  /// Numeric values in the range could not be parsed (invalid numbers).
  ParseError,
}

impl Responder for RangeError {
  fn into_response(self) -> crate::types::Response {
    match self {
      RangeError::InvalidFormat => (
        StatusCode::RANGE_NOT_SATISFIABLE,
        "Invalid Range format. Expected: bytes=start-end[,start-end...]",
      )
        .into_response(),
      RangeError::ParseError => (
        StatusCode::RANGE_NOT_SATISFIABLE,
        "Failed to parse numeric values from Range",
      )
        .into_response(),
    }
  }
}

fn parse_one(raw: &str) -> Result<RangeSpec, RangeError> {
  let raw = raw.trim();
  let Some((start_str, end_str)) = raw.split_once('-') else {
    return Err(RangeError::InvalidFormat);
  };
  let start_str = start_str.trim();
  let end_str = end_str.trim();

  match (start_str.is_empty(), end_str.is_empty()) {
    (true, true) => Err(RangeError::InvalidFormat),
    (true, false) => {
      let length = end_str.parse::<u64>().map_err(|_| RangeError::ParseError)?;
      Ok(RangeSpec::Suffix { length })
    }
    (false, true) => {
      let start = start_str
        .parse::<u64>()
        .map_err(|_| RangeError::ParseError)?;
      Ok(RangeSpec::From { start })
    }
    (false, false) => {
      let start = start_str
        .parse::<u64>()
        .map_err(|_| RangeError::ParseError)?;
      let end = end_str.parse::<u64>().map_err(|_| RangeError::ParseError)?;
      if start > end {
        return Err(RangeError::InvalidFormat);
      }
      Ok(RangeSpec::Inclusive { start, end })
    }
  }
}

/// Most specs accepted in one header, bounding the work per request as RFC 9110 §14.2 allows.
pub const MAX_RANGE_SPECS: usize = 100;

impl Range {
  /// Parses `bytes=start-end[,start-end...]`, or returns `Ok(None)` without a header; more
  /// than [`MAX_RANGE_SPECS`] specs is [`RangeError::InvalidFormat`].
  pub fn from_headers(headers: &HeaderMap) -> Result<Option<Self>, RangeError> {
    let value = match headers.get("range") {
      Some(v) => v.to_str().map_err(|_| RangeError::InvalidFormat)?,
      None => return Ok(None),
    };

    let Some(rest) = value.strip_prefix("bytes=") else {
      return Err(RangeError::InvalidFormat);
    };

    let mut specs = Vec::new();
    for part in rest.split(',') {
      if part.trim().is_empty() {
        continue;
      }
      if specs.len() >= MAX_RANGE_SPECS {
        return Err(RangeError::InvalidFormat);
      }
      specs.push(parse_one(part)?);
    }
    if specs.is_empty() {
      return Err(RangeError::InvalidFormat);
    }
    Ok(Some(Self { specs }))
  }
}

impl<'a> FromRequest<'a> for Option<Range> {
  type Error = RangeError;
  const ENTRIES: Entries = Entries::NONE;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Range::from_headers(req.headers()))
  }
}

impl<'a> FromRequestParts<'a> for Option<Range> {
  type Error = RangeError;
  const ENTRIES: Entries = Entries::NONE;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Range::from_headers(&parts.headers))
  }
}

#[cfg(test)]
mod tests {
  use http::HeaderMap;

  use super::*;

  fn headers(value: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert("range", value.parse().unwrap());
    h
  }

  #[test]
  fn parses_inclusive_range() {
    let r = Range::from_headers(&headers("bytes=0-99"))
      .unwrap()
      .unwrap();
    assert_eq!(r.specs.len(), 1);
    assert_eq!(r.specs[0], RangeSpec::Inclusive { start: 0, end: 99 });
  }

  #[test]
  fn parses_open_ended_range() {
    let r = Range::from_headers(&headers("bytes=100-"))
      .unwrap()
      .unwrap();
    assert_eq!(r.specs[0], RangeSpec::From { start: 100 });
  }

  #[test]
  fn parses_suffix_range() {
    let r = Range::from_headers(&headers("bytes=-500"))
      .unwrap()
      .unwrap();
    assert_eq!(r.specs[0], RangeSpec::Suffix { length: 500 });
  }

  #[test]
  fn parses_multi_range() {
    let r = Range::from_headers(&headers("bytes=0-100,200-300,400-"))
      .unwrap()
      .unwrap();
    assert_eq!(r.specs.len(), 3);
    assert_eq!(r.specs[0], RangeSpec::Inclusive { start: 0, end: 100 });
    assert_eq!(
      r.specs[1],
      RangeSpec::Inclusive {
        start: 200,
        end: 300
      }
    );
    assert_eq!(r.specs[2], RangeSpec::From { start: 400 });
  }

  #[test]
  fn rejects_inverted_range() {
    assert!(matches!(
      Range::from_headers(&headers("bytes=100-50")),
      Err(RangeError::InvalidFormat)
    ));
  }

  #[test]
  fn resolves_against_total() {
    let total = 1000;
    assert_eq!(
      RangeSpec::Inclusive { start: 0, end: 99 }.resolve(total),
      Some((0, 99))
    );
    assert_eq!(
      RangeSpec::From { start: 950 }.resolve(total),
      Some((950, 999))
    );
    assert_eq!(
      RangeSpec::Suffix { length: 200 }.resolve(total),
      Some((800, 999))
    );
    // Suffix longer than total clamps to the whole file.
    assert_eq!(
      RangeSpec::Suffix { length: 2000 }.resolve(total),
      Some((0, 999))
    );
  }
}
