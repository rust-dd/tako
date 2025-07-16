//! Accept-Language header parsing and locale preference extraction for internationalization.
//!
//! This module provides extractors for parsing HTTP Accept-Language headers to determine
//! client language preferences. It supports quality values (q-values) as defined in RFC 7231
//! and automatically sorts preferences by quality and order. The extractors enable easy
//! internationalization by providing structured access to client language preferences
//! with proper fallback handling.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::acc_lang::{AcceptLanguage, LanguagePreference};
//! use tako::extractors::FromRequestParts;
//! use http::request::Parts;
//!
//! async fn handler(accept_lang: AcceptLanguage) -> String {
//!     if let Some(preferred) = accept_lang.preferred() {
//!         format!("Preferred language: {} (quality: {})",
//!                 preferred.language, preferred.quality)
//!     } else {
//!         "No language preferences found".to_string()
//!     }
//! }
//!
//! // Check for specific language support
//! let accept = AcceptLanguage::parse_accept_language("en-US,en;q=0.9,es;q=0.8").unwrap();
//! assert!(accept.accepts("en-US"));
//! assert!(accept.accepts("es"));
//! ```

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
///
/// # Examples
///
/// ```rust
/// use tako::extractors::acc_lang::LanguagePreference;
///
/// let pref = LanguagePreference {
///     language: "en-US".to_string(),
///     quality: 0.9,
/// };
///
/// assert_eq!(pref.language, "en-US");
/// assert_eq!(pref.quality, 0.9);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct LanguagePreference {
    /// Language tag (e.g., "en-US", "fr", "zh-CN") following RFC 5646.
    pub language: String,
    /// Quality value from 0.0 to 1.0 indicating preference strength (default: 1.0).
    pub quality: f32,
}

/// Accept-Language header extractor for determining client language preferences.
///
/// Parses the Accept-Language header and provides access to client language preferences
/// sorted by quality value and order of appearance. This enables applications to
/// implement proper internationalization by selecting the most appropriate language
/// or locale based on client preferences.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::acc_lang::AcceptLanguage;
/// use tako::extractors::FromRequest;
/// use tako::types::Request;
///
/// async fn i18n_handler(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
///     let accept_lang = AcceptLanguage::from_request(&mut req).await?;
///
///     // Get the most preferred language
///     if let Some(preferred) = accept_lang.preferred() {
///         Ok(format!("Welcome! Language: {}", preferred.language))
///     } else {
///         Ok("Welcome! (Default language)".to_string())
///     }
/// }
///
/// // Check for specific language support
/// let header = "en-US,en;q=0.9,fr;q=0.8,*;q=0.1";
/// let accept = AcceptLanguage::parse_accept_language(header).unwrap();
/// assert!(accept.accepts("en-US"));
/// assert!(accept.accepts("fr"));
/// ```
#[derive(Debug, Clone)]
pub struct AcceptLanguage {
    /// Language preferences sorted by quality (highest first), then by order.
    pub languages: Vec<LanguagePreference>,
}

/// Error types for Accept-Language header extraction and parsing.
///
/// These errors cover various failure modes when parsing Accept-Language headers,
/// from missing headers to malformed quality values. Each error provides specific
/// information about what went wrong during parsing.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::acc_lang::{AcceptLanguage, AcceptLanguageError};
///
/// // This will fail due to invalid quality value
/// let result = AcceptLanguage::parse_accept_language("en;q=1.5");
/// match result {
///     Err(AcceptLanguageError::ParseError(_)) => println!("Invalid quality value"),
///     _ => unreachable!(),
/// }
/// ```
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
    ///
    /// Returns 400 Bad Request responses with descriptive error messages to help
    /// clients understand what went wrong with their Accept-Language header.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::acc_lang::AcceptLanguageError;
    /// use tako::responder::Responder;
    /// use http::StatusCode;
    ///
    /// let error = AcceptLanguageError::MissingHeader;
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::acc_lang::AcceptLanguage;
    ///
    /// let accept = AcceptLanguage::new();
    /// assert!(accept.languages.is_empty());
    /// assert!(accept.preferred().is_none());
    /// ```
    pub fn new() -> Self {
        Self {
            languages: Vec::new(),
        }
    }

    /// Gets the most preferred language based on quality values and order.
    ///
    /// Returns the language with the highest quality value, or the first language
    /// if multiple languages have the same highest quality value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::acc_lang::AcceptLanguage;
    ///
    /// let accept = AcceptLanguage::parse_accept_language("en;q=0.8,fr;q=0.9").unwrap();
    /// let preferred = accept.preferred().unwrap();
    /// assert_eq!(preferred.language, "fr");
    /// assert_eq!(preferred.quality, 0.9);
    /// ```
    pub fn preferred(&self) -> Option<&LanguagePreference> {
        self.languages.first()
    }

    /// Gets all language preferences in order of preference.
    ///
    /// Returns a slice of all language preferences sorted by quality value
    /// (highest first), with order of appearance as tiebreaker.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::acc_lang::AcceptLanguage;
    ///
    /// let accept = AcceptLanguage::parse_accept_language("en,fr;q=0.8,de;q=0.9").unwrap();
    /// let prefs = accept.preferences();
    /// assert_eq!(prefs.len(), 3);
    /// assert_eq!(prefs[0].language, "en");     // q=1.0 (default)
    /// assert_eq!(prefs[1].language, "de");     // q=0.9
    /// assert_eq!(prefs[2].language, "fr");     // q=0.8
    /// ```
    pub fn preferences(&self) -> &[LanguagePreference] {
        &self.languages
    }

    /// Checks if a specific language is accepted by the client.
    ///
    /// Returns true if the specified language appears in the client's
    /// Accept-Language header, regardless of quality value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::acc_lang::AcceptLanguage;
    ///
    /// let accept = AcceptLanguage::parse_accept_language("en-US,en;q=0.9,es;q=0.8").unwrap();
    /// assert!(accept.accepts("en-US"));
    /// assert!(accept.accepts("en"));
    /// assert!(accept.accepts("es"));
    /// assert!(!accept.accepts("fr"));
    /// ```
    pub fn accepts(&self, language: &str) -> bool {
        self.languages.iter().any(|pref| pref.language == language)
    }

    /// Extracts Accept-Language preferences from HTTP headers.
    ///
    /// Parses the Accept-Language header and returns a sorted list of language
    /// preferences. This is used internally by the FromRequest implementations.
    fn extract_from_headers(headers: &http::HeaderMap) -> Result<Self, AcceptLanguageError> {
        let header_value = headers
            .get("Accept-Language")
            .ok_or(AcceptLanguageError::MissingHeader)?;

        let header_str = header_value
            .to_str()
            .map_err(|_| AcceptLanguageError::InvalidHeader)?;

        Self::parse_accept_language(header_str)
    }

    /// Parses an Accept-Language header value into structured preferences.
    ///
    /// Handles the full Accept-Language syntax including quality values, multiple
    /// languages, and proper sorting by preference. Quality values are validated
    /// to be between 0.0 and 1.0 as per RFC 7231.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if quality values are invalid or malformed.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::acc_lang::AcceptLanguage;
    ///
    /// // Simple language list
    /// let accept = AcceptLanguage::parse_accept_language("en,fr,de").unwrap();
    /// assert_eq!(accept.languages.len(), 3);
    ///
    /// // With quality values
    /// let accept = AcceptLanguage::parse_accept_language("en;q=0.8,fr;q=0.9").unwrap();
    /// assert_eq!(accept.preferred().unwrap().language, "fr");
    ///
    /// // Complex header with wildcards
    /// let accept = AcceptLanguage::parse_accept_language("en-US,en;q=0.9,*;q=0.1").unwrap();
    /// assert_eq!(accept.languages.len(), 3);
    /// ```
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
    /// Creates an AcceptLanguage with no preferences.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::acc_lang::AcceptLanguage;
    ///
    /// let accept = AcceptLanguage::default();
    /// assert!(accept.languages.is_empty());
    /// ```
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> FromRequest<'a> for AcceptLanguage {
    type Error = AcceptLanguageError;

    /// Extracts Accept-Language preferences from the complete HTTP request.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::acc_lang::AcceptLanguage;
    /// use tako::extractors::FromRequest;
    /// use tako::types::Request;
    ///
    /// async fn handler(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
    ///     let accept_lang = AcceptLanguage::from_request(&mut req).await?;
    ///     if let Some(pref) = accept_lang.preferred() {
    ///         Ok(format!("Preferred: {}", pref.language))
    ///     } else {
    ///         Ok("No preferences".to_string())
    ///     }
    /// }
    /// ```
    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_headers(req.headers()))
    }
}

impl<'a> FromRequestParts<'a> for AcceptLanguage {
    type Error = AcceptLanguageError;

    /// Extracts Accept-Language preferences from HTTP request parts.
    ///
    /// This is more efficient when you only need headers and don't require
    /// access to the request body.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::acc_lang::AcceptLanguage;
    /// use tako::extractors::FromRequestParts;
    /// use http::request::Parts;
    ///
    /// async fn handler_parts(accept_lang: AcceptLanguage) -> String {
    ///     format!("Languages: {}", accept_lang.languages.len())
    /// }
    /// ```
    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_headers(&parts.headers))
    }
}
