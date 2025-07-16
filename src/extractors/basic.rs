//! Basic HTTP authentication credential extraction from Authorization headers.
//!
//! This module provides extractors for parsing HTTP Basic authentication credentials
//! as defined in RFC 7617. It extracts and validates the Authorization header with
//! Basic scheme, decodes the Base64-encoded credentials, and provides structured
//! access to username and password. The extractor handles proper error cases and
//! provides detailed error information for authentication failures.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::basic::Basic;
//! use tako::extractors::FromRequest;
//! use tako::types::Request;
//!
//! async fn protected_handler(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
//!     let basic_auth = Basic::from_request(&mut req).await?;
//!
//!     // Validate credentials (in production, check against database/LDAP/etc.)
//!     if basic_auth.username == "admin" && basic_auth.password == "secret" {
//!         Ok(format!("Welcome, {}!", basic_auth.username))
//!     } else {
//!         Ok("Invalid credentials".to_string())
//!     }
//! }
//!
//! // Usage in middleware or handlers
//! async fn auth_middleware_example(basic: Basic) -> String {
//!     format!("Authenticated user: {}", basic.username)
//! }
//! ```

use base64::{Engine, engine::general_purpose::STANDARD};
use http::{StatusCode, request::Parts};
use std::future::ready;

use crate::{
    extractors::{FromRequest, FromRequestParts},
    responder::Responder,
    types::Request,
};

/// Basic HTTP authentication credentials extracted from Authorization header.
///
/// Represents the username and password extracted from a Basic authentication
/// Authorization header. The credentials are Base64-decoded and split on the
/// first colon character as per RFC 7617. The raw token is also preserved
/// for logging or advanced use cases.
pub struct Basic {
    /// Username extracted from the Basic auth token.
    pub username: String,
    /// Password extracted from the Basic auth token.
    pub password: String,
    /// Raw Basic auth token as received in the Authorization header.
    pub raw: String,
}

/// Error types for Basic authentication extraction and validation.
#[derive(Debug)]
pub enum BasicAuthError {
    /// Authorization header is missing from the request.
    MissingAuthHeader,
    /// Authorization header contains invalid UTF-8 or cannot be parsed.
    InvalidAuthHeader,
    /// Authorization header does not use Basic authentication scheme.
    InvalidBasicFormat,
    /// Base64 encoding in the Basic auth token is invalid.
    InvalidBase64,
    /// Decoded credentials contain invalid UTF-8 characters.
    InvalidUtf8,
    /// Credentials format is invalid (missing colon separator).
    InvalidCredentialsFormat,
}

impl Responder for BasicAuthError {
    /// Converts Basic authentication errors into appropriate HTTP responses.
    fn into_response(self) -> crate::types::Response {
        let (status, message) = match self {
            BasicAuthError::MissingAuthHeader => {
                (StatusCode::UNAUTHORIZED, "Missing Authorization header")
            }
            BasicAuthError::InvalidAuthHeader => {
                (StatusCode::UNAUTHORIZED, "Invalid Authorization header")
            }
            BasicAuthError::InvalidBasicFormat => (
                StatusCode::UNAUTHORIZED,
                "Authorization header is not Basic auth",
            ),
            BasicAuthError::InvalidBase64 => (
                StatusCode::UNAUTHORIZED,
                "Invalid Base64 encoding in Basic auth",
            ),
            BasicAuthError::InvalidUtf8 => (
                StatusCode::UNAUTHORIZED,
                "Invalid UTF-8 in Basic auth credentials",
            ),
            BasicAuthError::InvalidCredentialsFormat => (
                StatusCode::UNAUTHORIZED,
                "Invalid credentials format in Basic auth",
            ),
        };
        (status, message).into_response()
    }
}

impl Basic {
    /// Parses Basic authentication credentials from HTTP headers.
    fn extract_from_headers(headers: &http::HeaderMap) -> Result<Self, BasicAuthError> {
        let auth_header = headers
            .get("Authorization")
            .ok_or(BasicAuthError::MissingAuthHeader)?;

        let auth_str = auth_header
            .to_str()
            .map_err(|_| BasicAuthError::InvalidAuthHeader)?;

        if !auth_str.starts_with("Basic ") {
            return Err(BasicAuthError::InvalidBasicFormat);
        }

        let encoded = &auth_str[6..];
        let decoded = STANDARD
            .decode(encoded)
            .map_err(|_| BasicAuthError::InvalidBase64)?;

        let decoded_str = std::str::from_utf8(&decoded).map_err(|_| BasicAuthError::InvalidUtf8)?;

        let parts: Vec<&str> = decoded_str.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(BasicAuthError::InvalidCredentialsFormat);
        }

        Ok(Basic {
            username: parts[0].to_string(),
            password: parts[1].to_string(),
            raw: auth_str.to_string(),
        })
    }
}

impl<'a> FromRequest<'a> for Basic {
    type Error = BasicAuthError;

    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_headers(req.headers()))
    }
}

impl<'a> FromRequestParts<'a> for Basic {
    type Error = BasicAuthError;

    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_headers(&parts.headers))
    }
}
