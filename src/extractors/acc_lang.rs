//! Accept-Language header parsing and locale preference extraction for internationalization.
//!
//! This module provides extractors for parsing HTTP Accept-Language headers to determine
//! client language preferences. It supports quality values (q-values) as defined in RFC 7231
//! and automatically sorts preferences by quality and order. The extractors enable easy
//! internationalization by providing structured access to client language preferences
//! with proper fallback handling.
//!

use http::{StatusCode, request::Parts};
use std::future::ready;

use crate::{
    extractors::{FromRequest, FromRequestParts},
    responder::Responder,
    types::Request,
};

/// Language preference with quality value from Accept-Language header.
///
/// Represents a single language preference as specified in RFC 7231, including
/// the language tag and its associated quality value. Quality values range from
/// 0.0 to 1.0, with 1.0 being the default when no quality is specified.
#[derive(Debug, Clone, PartialEq)]
pub struct LanguagePreference {
    /// Language tag (e.g., "en-US", "fr", "zh-CN") following RFC 5646.
    pub language: String,
    /// Quality value from 0.0 to 1.0 indicating preference strength (default: 1.0).
    pub quality: f32,
}

/// Accept-Language header extractor for determining client language preferences.
#[derive(Debug, Clone)]
pub struct AcceptLanguage {
    /// Language preferences sorted by quality (highest first), then by order.
    pub languages: Vec<LanguagePreference>,
}

/// Error types for Accept-Language header extraction and parsing.
#[derive(Debug)]
pub enum AcceptLanguageError {
    /// Accept-Language header is missing from the request.
    MissingHeader,
    /// Accept-Language header contains invalid UTF-8 or cannot be parsed as text.
    InvalidHeader,
    /// Failed to parse header value (contains specific error details).
    ParseError(String),
}

impl Responder for AcceptLanguageError {
    /// Converts Accept-Language errors into appropriate HTTP error responses.
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
    /// Creates a new empty AcceptLanguage with no preferences.
    pub fn new() -> Self {
        Self {
            languages: Vec::new(),
        }
    }

    /// Returns the most preferred language based on quality values and order.
    pub fn preferred(&self) -> Option<&LanguagePreference> {
        self.languages.first()
    }

    /// Returns all language preferences in order of preference.
    pub fn preferences(&self) -> &[LanguagePreference] {
        &self.languages
    }

    /// Verifies if a specific language is accepted by the client.
    pub fn accepts(&self, language: &str) -> bool {
        self.languages.iter().any(|pref| pref.language == language)
    }

    /// Parses Accept-Language preferences from HTTP headers.
    fn extract_from_headers(headers: &http::HeaderMap) -> Result<Self, AcceptLanguageError> {
        let header_value = headers
            .get("Accept-Language")
            .ok_or(AcceptLanguageError::MissingHeader)?;

        let header_str = header_value
            .to_str()
            .map_err(|_| AcceptLanguageError::InvalidHeader)?;

        Self::parse_accept_language(header_str)
    }

    /// Converts an Accept-Language header value into structured preferences.
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
    /// Initializes an AcceptLanguage instance with no preferences.
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
