use http::{StatusCode, request::Parts};
use std::future::ready;

use crate::{
    extractors::{FromRequest, FromRequestParts},
    responder::Responder,
    types::Request,
};

/// Represents a language preference with its quality value.
#[derive(Debug, Clone, PartialEq)]
pub struct LanguagePreference {
    /// The language tag (e.g., "en-US", "fr", "de")
    pub language: String,
    /// Quality value from 0.0 to 1.0 (default is 1.0)
    pub quality: f32,
}

/// Extractor for the Accept-Language header.
#[derive(Debug, Clone)]
pub struct AcceptLanguage {
    /// Languages in order of preference (highest quality first)
    pub languages: Vec<LanguagePreference>,
}

/// Error type for Accept-Language extraction.
#[derive(Debug)]
pub enum AcceptLanguageError {
    MissingHeader,
    InvalidHeader,
    ParseError(String),
}

impl Responder for AcceptLanguageError {
    fn into_response(self) -> crate::types::Response {
        match self {
            AcceptLanguageError::MissingHeader => {
                (StatusCode::BAD_REQUEST, "Missing Accept-Language header").into_response()
            }
            AcceptLanguageError::InvalidHeader => {
                (StatusCode::BAD_REQUEST, "Invalid Accept-Language header").into_response()
            }
            AcceptLanguageError::ParseError(err) => (
                StatusCode::BAD_REQUEST,
                format!("Failed to parse Accept-Language header: {}", err),
            )
                .into_response(),
        }
    }
}

impl AcceptLanguage {
    /// Creates a new AcceptLanguage with no preferences.
    pub fn new() -> Self {
        Self {
            languages: Vec::new(),
        }
    }

    /// Gets the most preferred language.
    pub fn preferred(&self) -> Option<&LanguagePreference> {
        self.languages.first()
    }

    /// Gets all languages in preference order.
    pub fn preferences(&self) -> &[LanguagePreference] {
        &self.languages
    }

    /// Checks if a specific language is accepted.
    pub fn accepts(&self, language: &str) -> bool {
        self.languages.iter().any(|pref| pref.language == language)
    }

    /// Extracts Accept-Language from headers.
    fn extract_from_headers(headers: &http::HeaderMap) -> Result<Self, AcceptLanguageError> {
        let header_value = headers
            .get("Accept-Language")
            .ok_or(AcceptLanguageError::MissingHeader)?;

        let header_str = header_value
            .to_str()
            .map_err(|_| AcceptLanguageError::InvalidHeader)?;

        Self::parse_accept_language(header_str)
    }

    /// Parses the Accept-Language header value.
    fn parse_accept_language(header_value: &str) -> Result<Self, AcceptLanguageError> {
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
                    AcceptLanguageError::ParseError(format!(
                        "Invalid quality value '{}': {}",
                        quality_str, e
                    ))
                })?;

                if quality < 0.0 || quality > 1.0 {
                    return Err(AcceptLanguageError::ParseError(format!(
                        "Quality value must be between 0.0 and 1.0, got: {}",
                        quality
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

        // Sort by quality (highest first), then by order of appearance for equal qualities
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

    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_headers(req.headers()))
    }
}

impl<'a> FromRequestParts<'a> for AcceptLanguage {
    type Error = AcceptLanguageError;

    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_headers(&parts.headers))
    }
}
