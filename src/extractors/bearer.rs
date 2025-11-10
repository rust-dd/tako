//! Bearer token authentication extraction from Authorization headers.
//!
//! This module provides extractors for parsing HTTP Bearer token authentication
//! as defined in RFC 6750. It extracts and validates the Authorization header with
//! Bearer scheme, providing structured access to the token value. This is commonly
//! used for API authentication with JWT tokens, OAuth access tokens, or custom
//! authentication schemes that use bearer tokens.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::bearer::Bearer;
//! use tako::extractors::FromRequest;
//! use tako::types::Request;
//!
//! async fn api_handler(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
//!     let bearer = Bearer::from_request(&mut req).await?;
//!
//!     // Validate token (in production, verify JWT or check against database)
//!     if is_valid_token(&bearer.token) {
//!         Ok(format!("Access granted with token: {}...", &bearer.token[..8]))
//!     } else {
//!         Ok("Invalid token".to_string())
//!     }
//! }
//!
//! fn is_valid_token(token: &str) -> bool {
//!     // In production, verify JWT signature, check expiration, etc.
//!     token.len() > 10 && token.starts_with("eyJ") // Simple JWT check
//! }
//!
//! // Usage in middleware
//! async fn auth_middleware(bearer: Bearer) -> String {
//!     format!("Authenticated with token ending in: ...{}",
//!             &bearer.token[bearer.token.len().saturating_sub(4)..])
//! }
//! ```

use http::{StatusCode, request::Parts};
use std::future::ready;

use crate::{
  extractors::{FromRequest, FromRequestParts},
  responder::Responder,
  types::Request,
};

/// Bearer token authentication credentials extracted from Authorization header.
///
/// Represents the Bearer token extracted from an HTTP Authorization header. The token
/// is extracted without the "Bearer " prefix for easy use in authentication logic.
/// The original header value is preserved for logging or advanced use cases where
/// the complete Authorization header is needed.
pub struct Bearer {
  /// Token value extracted from Bearer auth header (without "Bearer " prefix).
  pub token: String,
  /// Complete Bearer token string as received ("Bearer " + token).
  pub with_bearer: String,
}

/// Error types for Bearer token authentication extraction and validation.
#[derive(Debug)]
pub enum BearerAuthError {
  /// Authorization header is missing from the request.
  MissingAuthHeader,
  /// Authorization header contains invalid UTF-8 or cannot be parsed.
  InvalidAuthHeader,
  /// Authorization header does not use Bearer authentication scheme.
  InvalidBearerFormat,
  /// Bearer token is present but empty.
  EmptyToken,
}

impl Responder for BearerAuthError {
  /// Converts Bearer authentication errors into appropriate HTTP responses.
  fn into_response(self) -> crate::types::Response {
    let (status, message) = match self {
      BearerAuthError::MissingAuthHeader => {
        (StatusCode::UNAUTHORIZED, "Missing Authorization header")
      }
      BearerAuthError::InvalidAuthHeader => {
        (StatusCode::UNAUTHORIZED, "Invalid Authorization header")
      }
      BearerAuthError::InvalidBearerFormat => (
        StatusCode::UNAUTHORIZED,
        "Authorization header is not Bearer token",
      ),
      BearerAuthError::EmptyToken => (StatusCode::UNAUTHORIZED, "Bearer token is empty"),
    };
    (status, message).into_response()
  }
}

impl Bearer {
  /// Parses Bearer token from HTTP headers.
  fn extract_from_headers(headers: &http::HeaderMap) -> Result<Self, BearerAuthError> {
    let auth_header = headers
      .get("Authorization")
      .ok_or(BearerAuthError::MissingAuthHeader)?;

    let auth_str = auth_header
      .to_str()
      .map_err(|_| BearerAuthError::InvalidAuthHeader)?;

    if !auth_str.starts_with("Bearer ") {
      return Err(BearerAuthError::InvalidBearerFormat);
    }

    let token = &auth_str[7..];
    if token.is_empty() {
      return Err(BearerAuthError::EmptyToken);
    }

    Ok(Bearer {
      token: token.to_string(),
      with_bearer: auth_str.to_string(),
    })
  }
}

impl<'a> FromRequest<'a> for Bearer {
  type Error = BearerAuthError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    ready(Self::extract_from_headers(req.headers()))
  }
}

impl<'a> FromRequestParts<'a> for Bearer {
  type Error = BearerAuthError;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    ready(Self::extract_from_headers(&parts.headers))
  }
}
