//! HTTP Basic credentials (RFC 7617) from the `Authorization` header.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::basic::Basic;
//! use tako::extractors::FromRequest;
//! use tako::types::Request;
//!
//! async fn protected_handler(mut req: Request) -> Result<String, tako::extractors::basic::BasicAuthError> {
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

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use http::StatusCode;
use http::request::Parts;
use tako_rs_core::extractors::Entries;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;

/// Decoded Basic credentials, split at the first `:` so the password may contain colons.
pub struct Basic {
  pub username: String,
  pub password: String,
  /// The whole header value, including the `Basic ` prefix.
  pub raw: String,
}

/// Rejection of [`Basic`]; every variant responds 401.
#[derive(Debug)]
pub enum BasicAuthError {
  MissingAuthHeader,
  /// The header value is not visible ASCII.
  InvalidAuthHeader,
  /// The value does not start with a case-insensitive `Basic ` scheme.
  InvalidBasicFormat,
  InvalidBase64,
  InvalidUtf8,
  /// The decoded credentials contain no `:`.
  InvalidCredentialsFormat,
}

impl Responder for BasicAuthError {
  fn into_response(self) -> tako_rs_core::types::Response {
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
  fn extract_from_headers(headers: &http::HeaderMap) -> Result<Self, BasicAuthError> {
    let auth_header = headers
      .get("Authorization")
      .ok_or(BasicAuthError::MissingAuthHeader)?;

    let auth_str = auth_header
      .to_str()
      .map_err(|_| BasicAuthError::InvalidAuthHeader)?;

    if !auth_str
      .get(..6)
      .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Basic "))
    {
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
  const ENTRIES: Entries = Entries::NONE;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from_headers(req.headers()))
  }
}

impl<'a> FromRequestParts<'a> for Basic {
  type Error = BasicAuthError;
  const ENTRIES: Entries = Entries::NONE;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from_headers(&parts.headers))
  }
}
