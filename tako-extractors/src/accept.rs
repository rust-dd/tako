//! Content negotiation extractor for parsing the `Accept` header.
//!
//! Provides the `Accept` extractor which parses the `Accept` header and
//! exposes methods to check which content types the client prefers.
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

use tako_core::extractors::FromRequestParts;

/// Parsed Accept header with content negotiation helpers.
#[derive(Debug, Clone)]
pub struct Accept {
  /// Parsed media types with their quality values, sorted by preference.
  media_types: Vec<MediaType>,
}

/// A single media type entry from the Accept header.
#[derive(Debug, Clone)]
struct MediaType {
  essence: String,
  quality: f32,
}

impl Accept {
  /// Returns true if the given media type is preferred (has highest quality for its type).
  pub fn prefers(&self, media_type: &str) -> bool {
    self
      .media_types
      .first()
      .is_some_and(|mt| mt.essence == media_type || mt.essence == "*/*")
  }

  /// Returns true if the client accepts the given media type.
  pub fn accepts(&self, media_type: &str) -> bool {
    self.media_types.iter().any(|mt| {
      mt.essence == media_type
        || mt.essence == "*/*"
        || (mt.essence.ends_with("/*")
          && media_type.starts_with(mt.essence.trim_end_matches("/*")))
    })
  }

  /// Returns the most preferred media type, if any.
  pub fn preferred(&self) -> Option<&str> {
    self.media_types.first().map(|mt| mt.essence.as_str())
  }

  /// Returns all accepted media types sorted by quality (highest first).
  pub fn types(&self) -> Vec<&str> {
    self.media_types.iter().map(|mt| mt.essence.as_str()).collect()
  }
}

fn parse_accept(header: &str) -> Vec<MediaType> {
  let mut types: Vec<MediaType> = header
    .split(',')
    .filter_map(|part| {
      let part = part.trim();
      if part.is_empty() {
        return None;
      }

      let mut quality = 1.0f32;
      let mut essence = part;

      if let Some(idx) = part.find(";q=") {
        essence = part[..idx].trim();
        if let Ok(q) = part[idx + 3..].trim().parse::<f32>() {
          quality = q;
        }
      } else if let Some(idx) = part.find(';') {
        essence = part[..idx].trim();
      }

      Some(MediaType {
        essence: essence.to_string(),
        quality,
      })
    })
    .collect();

  types.sort_by(|a, b| b.quality.partial_cmp(&a.quality).unwrap_or(std::cmp::Ordering::Equal));
  types
}

impl<'a> FromRequestParts<'a> for Accept {
  type Error = std::convert::Infallible;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    let accept_header = parts
      .headers
      .get(http::header::ACCEPT)
      .and_then(|v| v.to_str().ok())
      .unwrap_or("*/*");

    let media_types = parse_accept(accept_header);

    futures_util::future::ready(Ok(Accept { media_types }))
  }
}
