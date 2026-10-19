//! `Accept` header content negotiation; a missing header means `*/*`.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::accept::Accept;
//! use tako::responder::Responder;
//! use tako::types::Request;
//!
//! async fn handler(accept: Accept, _req: Request) -> impl Responder {
//!     if accept.prefers("application/json") {
//!         r#"{"message": "hello"}"#.to_string()
//!     } else {
//!         "hello".to_string()
//!     }
//! }
//! ```

use http::request::Parts;
use tako_rs_core::extractors::Entries;
use tako_rs_core::extractors::FromRequestParts;

/// The parsed `Accept` header, best quality first; extraction never fails.
#[derive(Debug, Clone)]
pub struct Accept {
  media_types: Vec<MediaType>,
}

#[derive(Debug, Clone)]
struct MediaType {
  essence: String,
  media_range: String,
  quality: f32,
  parameters: Vec<(String, String)>,
}

impl Accept {
  fn from_headers(headers: &http::HeaderMap) -> Self {
    let mut values = headers
      .get_all(http::header::ACCEPT)
      .iter()
      .filter_map(|value| value.to_str().ok())
      .peekable();
    let media_types = if values.peek().is_none() {
      parse_accept(["*/*"])
    } else {
      parse_accept(values)
    };
    Self { media_types }
  }

  /// Whether the effective quality of `media_type` is positive and matches the best entry.
  pub fn prefers(&self, media_type: &str) -> bool {
    let quality = self.quality(media_type);
    quality > 0.0
      && self
        .media_types
        .first()
        .is_some_and(|mt| quality >= mt.quality)
  }

  /// Whether the most specific matching range permits `media_type` with a positive quality.
  pub fn accepts(&self, media_type: &str) -> bool {
    self.quality(media_type) > 0.0
  }

  fn quality(&self, media_type: &str) -> f32 {
    let parsed = media_type.parse::<mime::Mime>().ok();
    let essence = parsed.as_ref().map_or(media_type, mime::Mime::essence_str);
    let mut best = None;
    for entry in &self.media_types {
      let specificity = if entry.essence.eq_ignore_ascii_case(essence) {
        2
      } else if entry.essence == "*/*" {
        0
      } else if entry.essence.strip_suffix("/*").is_some_and(|prefix| {
        essence
          .split_once('/')
          .is_some_and(|(kind, _)| kind.eq_ignore_ascii_case(prefix))
      }) {
        1
      } else {
        continue;
      };
      if !entry.parameters.iter().all(|(name, value)| {
        parsed
          .as_ref()
          .and_then(|media| media.get_param(name.as_str()))
          .is_some_and(|candidate| candidate == value.as_str())
      }) {
        continue;
      }
      let specificity = (specificity, entry.parameters.len());
      // A more specific exclusion must win even when a wildcard has a higher quality.
      if best.is_none_or(|(current, _)| specificity > current) {
        best = Some((specificity, entry.quality));
      }
    }
    best.map_or(0.0, |(_, quality)| quality)
  }

  /// The best-quality range with its media parameters, excluding `q`, or `None` if all are zero.
  pub fn preferred(&self) -> Option<&str> {
    self
      .media_types
      .iter()
      .find(|mt| mt.quality > 0.0)
      .map(|mt| mt.media_range.as_str())
  }

  /// Positive-quality ranges with their media parameters, excluding `q`, best quality first.
  pub fn types(&self) -> Vec<&str> {
    self
      .media_types
      .iter()
      .filter(|mt| mt.quality > 0.0)
      .map(|mt| mt.media_range.as_str())
      .collect()
  }
}

fn media_ranges(header: &str) -> impl Iterator<Item = &str> {
  let mut quoted = false;
  let mut escaped = false;
  header.split(move |character| {
    if escaped {
      escaped = false;
      return false;
    }
    match character {
      '\\' if quoted => escaped = true,
      '"' => quoted = !quoted,
      _ => {}
    }
    character == ',' && !quoted
  })
}

fn parse_accept<'a>(headers: impl IntoIterator<Item = &'a str>) -> Vec<MediaType> {
  let mut types = headers
    .into_iter()
    .flat_map(media_ranges)
    .filter_map(|part| {
      let media = part.trim().parse::<mime::Mime>().ok()?;
      let mut quality = 1.0f32;
      let mut parameters = Vec::new();
      let mut media_range = media.essence_str().to_owned();
      for (name, value) in media.params() {
        if name == "q" {
          let q = value.as_str().parse::<f32>().unwrap_or(0.0);
          quality = if q.is_nan() { 0.0 } else { q.clamp(0.0, 1.0) };
        } else {
          parameters.push((name.to_string(), value.to_string()));
          media_range.push(';');
          media_range.push_str(name.as_str());
          media_range.push('=');
          let quoted = !value
            .as_str()
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte));
          if quoted {
            media_range.push('"');
          }
          media_range.push_str(value.as_str());
          if quoted {
            media_range.push('"');
          }
        }
      }

      Some(MediaType {
        essence: media.essence_str().to_owned(),
        media_range,
        quality,
        parameters,
      })
    })
    .collect::<Vec<_>>();

  types.sort_by(|a, b| {
    b.quality
      .partial_cmp(&a.quality)
      .unwrap_or(std::cmp::Ordering::Equal)
  });
  types
}

impl<'a> FromRequestParts<'a> for Accept {
  type Error = std::convert::Infallible;
  const ENTRIES: Entries = Entries::NONE;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Ok(Self::from_headers(&parts.headers)))
  }
}

impl<'a> tako_rs_core::extractors::FromRequest<'a> for Accept {
  type Error = std::convert::Infallible;
  const ENTRIES: Entries = Entries::NONE;

  fn from_request(
    req: &'a mut tako_rs_core::types::Request,
  ) -> impl Future<Output = Result<Self, Self::Error>> + Send + 'a {
    std::future::ready(Ok(Self::from_headers(req.headers())))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn wildcard_matches_subtype_not_prefix() {
    let a = Accept {
      media_types: parse_accept(["image/*"]),
    };
    assert!(a.accepts("image/png"));
    assert!(a.accepts("image/svg+xml"));
    assert!(!a.accepts("imagezzz"));
    assert!(!a.accepts("imageX/png"));
  }

  #[test]
  fn quality_values_are_clamped() {
    let parsed = parse_accept(["a/a;q=999.0, b/b;q=0.5, c/c;q=NaN, d/d;q=-1.0"]);
    let qualities: Vec<f32> = parsed.iter().map(|mt| mt.quality).collect();
    for q in &qualities {
      assert!(*q >= 0.0 && *q <= 1.0, "quality {q} out of range");
    }
    // a/a (clamped to 1.0) wins; c/c and d/d both fall to 0.0.
    assert_eq!(parsed[0].essence, "a/a");
    assert!((parsed[0].quality - 1.0).abs() < 1e-6);
  }
}
