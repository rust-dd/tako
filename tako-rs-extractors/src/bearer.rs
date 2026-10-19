//! Bearer tokens (RFC 6750) from the `Authorization` header; the token is not validated.
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

use http::StatusCode;
use http::request::Parts;
use tako_rs_core::extractors::Entries;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;

/// A bearer token; `token` excludes the `Bearer ` prefix, `with_bearer` is the whole value.
pub struct Bearer {
  pub token: String,
  pub with_bearer: String,
}

/// Rejection of [`Bearer`]; every variant responds 401.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BearerAuthError {
  MissingAuthHeader,
  /// The header value is not visible ASCII.
  InvalidAuthHeader,
  /// The value does not start with a case-insensitive `Bearer ` scheme.
  InvalidBearerFormat,
  EmptyToken,
}

impl std::fmt::Display for BearerAuthError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::MissingAuthHeader => write!(f, "missing Authorization header"),
      Self::InvalidAuthHeader => write!(f, "invalid Authorization header"),
      Self::InvalidBearerFormat => write!(f, "Authorization header is not Bearer token"),
      Self::EmptyToken => write!(f, "Bearer token is empty"),
    }
  }
}

impl std::error::Error for BearerAuthError {}

impl Responder for BearerAuthError {
  fn into_response(self) -> tako_rs_core::types::Response {
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
  fn extract_from_headers(headers: &http::HeaderMap) -> Result<Self, BearerAuthError> {
    let auth_header = headers
      .get("Authorization")
      .ok_or(BearerAuthError::MissingAuthHeader)?;

    let auth_str = auth_header
      .to_str()
      .map_err(|_| BearerAuthError::InvalidAuthHeader)?;

    if !auth_str
      .get(..7)
      .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Bearer "))
    {
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
  const ENTRIES: Entries = Entries::NONE;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from_headers(req.headers()))
  }
}

impl<'a> FromRequestParts<'a> for Bearer {
  type Error = BearerAuthError;
  const ENTRIES: Entries = Entries::NONE;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from_headers(&parts.headers))
  }
}
