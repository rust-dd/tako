//! `Accept-Language` parsing with RFC 7231 quality values.

use http::StatusCode;
use http::request::Parts;
use tako_rs_core::extractors::Entries;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;

/// One `Accept-Language` entry.
#[derive(Debug, Clone, PartialEq)]
pub struct LanguagePreference {
  /// The language tag as the client sent it, such as `en-US`.
  pub language: String,
  /// In 0.0..=1.0; 1.0 when the entry has no `q`.
  pub quality: f32,
}

/// The parsed `Accept-Language` header; a missing or malformed header rejects with 400.
#[derive(Debug, Clone)]
pub struct AcceptLanguage {
  /// Highest quality first; equal qualities keep header order.
  pub languages: Vec<LanguagePreference>,
}

/// Rejection of [`AcceptLanguage`].
#[derive(Debug)]
pub enum AcceptLanguageError {
  MissingHeader,
  /// The header value is not visible ASCII.
  InvalidHeader,
  /// A `q` value is not a number in 0.0..=1.0.
  ParseError(String),
}

impl Responder for AcceptLanguageError {
  fn into_response(self) -> tako_rs_core::types::Response {
    match self {
      AcceptLanguageError::MissingHeader => {
        (StatusCode::BAD_REQUEST, "Missing Accept-Language header").into_response()
      }
      AcceptLanguageError::InvalidHeader => {
        (StatusCode::BAD_REQUEST, "Invalid Accept-Language header").into_response()
      }
      AcceptLanguageError::ParseError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Failed to parse Accept-Language header: {err}"),
      )
        .into_response(),
    }
  }
}

impl AcceptLanguage {
  /// An empty preference list.
  pub fn new() -> Self {
    Self {
      languages: Vec::new(),
    }
  }

  /// The highest-quality language; ties go to the earliest.
  pub fn preferred(&self) -> Option<&LanguagePreference> {
    self.languages.first()
  }

  /// All preferences, best first.
  pub fn preferences(&self) -> &[LanguagePreference] {
    &self.languages
  }

  /// Whether `language` is listed, compared exactly; an entry with `q=0` still counts.
  pub fn accepts(&self, language: &str) -> bool {
    self.languages.iter().any(|pref| pref.language == language)
  }

  fn extract_from_headers(headers: &http::HeaderMap) -> Result<Self, AcceptLanguageError> {
    let header_value = headers
      .get("Accept-Language")
      .ok_or(AcceptLanguageError::MissingHeader)?;

    let header_str = header_value
      .to_str()
      .map_err(|_| AcceptLanguageError::InvalidHeader)?;

    Self::parse_accept_language(header_str)
  }

  /// Parses a header value; a `q` outside 0.0..=1.0 is an error.
  pub fn parse_accept_language(header_value: &str) -> Result<Self, AcceptLanguageError> {
    let mut languages = Vec::new();

    for part in header_value.split(',') {
      let part = part.trim();
      if part.is_empty() {
        continue;
      }

      let (language, quality) = if let Some(q_pos) = part.find(";q=") {
        let language = part[..q_pos].trim().to_string();
        let quality_str = &part[q_pos + 3..].trim();

        let quality = quality_str.parse::<f32>().map_err(|e| {
          AcceptLanguageError::ParseError(format!("Invalid quality value '{quality_str}': {e}"))
        })?;

        if !(0.0..=1.0).contains(&quality) {
          return Err(AcceptLanguageError::ParseError(format!(
            "Quality value must be between 0.0 and 1.0, got: {quality}"
          )));
        }

        (language, quality)
      } else {
        (part.to_string(), 1.0)
      };

      if !language.is_empty() {
        languages.push(LanguagePreference { language, quality });
      }
    }

    // Stable, so equal qualities keep header order.
    languages.sort_by(|a, b| {
      b.quality
        .partial_cmp(&a.quality)
        .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(AcceptLanguage { languages })
  }
}

impl Default for AcceptLanguage {
  fn default() -> Self {
    Self::new()
  }
}

impl<'a> FromRequest<'a> for AcceptLanguage {
  type Error = AcceptLanguageError;
  const ENTRIES: Entries = Entries::NONE;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from_headers(req.headers()))
  }
}

impl<'a> FromRequestParts<'a> for AcceptLanguage {
  type Error = AcceptLanguageError;
  const ENTRIES: Entries = Entries::NONE;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from_headers(&parts.headers))
  }
}
