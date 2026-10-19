//! JWTs from `Authorization: Bearer`, decoded without verifying the signature or claims. For
//! verified claims, install `JwtAuth` middleware and extract `JwtClaimsVerified`.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::jwt::{Jwt, JwtClaimsUnverified};
//! use tako::extractors::FromRequest;
//! use tako::types::Request;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Debug, Deserialize, Serialize)]
//! struct UserClaims {
//!     sub: String,
//!     exp: u64,
//!     iat: u64,
//!     email: String,
//!     role: String,
//! }
//!
//! async fn protected_handler(mut req: Request) -> Result<String, tako::extractors::jwt::JwtError> {
//!     let jwt_claims: JwtClaimsUnverified<UserClaims> =
//!         JwtClaimsUnverified::from_request(&mut req).await?;
//!
//!     println!("User: {} ({})", jwt_claims.0.email, jwt_claims.0.role);
//!     Ok(format!("Welcome, {}!", jwt_claims.0.email))
//! }
//!
//! // Raw JWT token access
//! async fn jwt_handler(jwt: Jwt) -> String {
//!     format!("JWT token length: {}", jwt.token.len())
//! }
//! ```

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use http::StatusCode;
use http::request::Parts;
use serde::de::DeserializeOwned;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;

/// The bearer token, neither parsed nor verified.
#[doc(alias = "jwt")]
pub struct Jwt {
  pub token: String,
  /// The whole header value, including `Bearer `.
  pub header: String,
}

/// The claims, decoded but not verified: the signature, `exp` and `nbf` are unchecked, so treat
/// them as untrusted unless `JwtAuth` middleware ran first.
#[doc(alias = "jwt_claims_unverified")]
pub struct JwtClaimsUnverified<T>(pub T);

/// Rejection of [`Jwt`] and [`JwtClaimsUnverified`]; every variant responds 401.
#[derive(Debug)]
pub enum JwtError {
  MissingAuthHeader,
  /// The header value is not visible ASCII.
  InvalidAuthHeader,
  /// The value does not start with `Bearer ` (case-sensitive).
  InvalidBearerFormat,
  EmptyToken,
  /// The token is not three non-empty dot-separated parts.
  InvalidJwtFormat,
  /// The header part is not base64url-encoded JSON.
  InvalidJwtHeader,
  /// The claims part is not base64url-encoded JSON.
  InvalidJwtClaims,
  /// The signature part is not base64url.
  InvalidJwtSignature,
  ClaimsDeserializationError(String),
  TokenExpired,
  TokenNotYetValid,
}

impl Responder for JwtError {
  fn into_response(self) -> tako_rs_core::types::Response {
    let (status, message) = match self {
      JwtError::MissingAuthHeader => (StatusCode::UNAUTHORIZED, "Missing Authorization header"),
      JwtError::InvalidAuthHeader => (StatusCode::UNAUTHORIZED, "Invalid Authorization header"),
      JwtError::InvalidBearerFormat => (
        StatusCode::UNAUTHORIZED,
        "Authorization header is not Bearer token",
      ),
      JwtError::EmptyToken => (StatusCode::UNAUTHORIZED, "JWT token is empty"),
      JwtError::InvalidJwtFormat => (StatusCode::UNAUTHORIZED, "Invalid JWT token format"),
      JwtError::InvalidJwtHeader => (StatusCode::UNAUTHORIZED, "Invalid JWT header section"),
      JwtError::InvalidJwtClaims => (StatusCode::UNAUTHORIZED, "Invalid JWT claims section"),
      JwtError::InvalidJwtSignature => (StatusCode::UNAUTHORIZED, "Invalid JWT signature section"),
      JwtError::ClaimsDeserializationError(_) => (
        StatusCode::UNAUTHORIZED,
        "JWT claims deserialization failed",
      ),
      JwtError::TokenExpired => (StatusCode::UNAUTHORIZED, "JWT token has expired"),
      JwtError::TokenNotYetValid => (StatusCode::UNAUTHORIZED, "JWT token is not yet valid"),
    };
    (status, message).into_response()
  }
}

impl Jwt {
  fn extract_from_headers(headers: &http::HeaderMap) -> Result<Self, JwtError> {
    let auth_header = headers
      .get("Authorization")
      .ok_or(JwtError::MissingAuthHeader)?;

    let auth_str = auth_header
      .to_str()
      .map_err(|_| JwtError::InvalidAuthHeader)?;

    if !auth_str.starts_with("Bearer ") {
      return Err(JwtError::InvalidBearerFormat);
    }

    let token = &auth_str[7..];
    if token.is_empty() {
      return Err(JwtError::EmptyToken);
    }

    Ok(Jwt {
      token: token.to_string(),
      header: auth_str.to_string(),
    })
  }

  /// Checks for three non-empty dot-separated parts without decoding them.
  pub fn validate_format(&self) -> Result<(), JwtError> {
    let parts = self.token.split('.').collect::<Vec<&str>>();
    if parts.len() != 3 {
      return Err(JwtError::InvalidJwtFormat);
    }

    for part in &parts {
      if part.is_empty() {
        return Err(JwtError::InvalidJwtFormat);
      }
    }

    Ok(())
  }

  /// The decoded header JSON.
  pub fn header(&self) -> Result<serde_json::Value, JwtError> {
    let parts: Vec<&str> = self.token.split('.').collect();
    if parts.len() != 3 {
      return Err(JwtError::InvalidJwtFormat);
    }

    let header_bytes = URL_SAFE_NO_PAD
      .decode(parts[0])
      .map_err(|_| JwtError::InvalidJwtHeader)?;

    let header: serde_json::Value =
      serde_json::from_slice(&header_bytes).map_err(|_| JwtError::InvalidJwtHeader)?;

    Ok(header)
  }

  /// The decoded claims JSON, unverified.
  pub fn claims(&self) -> Result<serde_json::Value, JwtError> {
    let parts: Vec<&str> = self.token.split('.').collect();
    if parts.len() != 3 {
      return Err(JwtError::InvalidJwtFormat);
    }

    let claims_bytes = URL_SAFE_NO_PAD
      .decode(parts[1])
      .map_err(|_| JwtError::InvalidJwtClaims)?;

    let claims: serde_json::Value =
      serde_json::from_slice(&claims_bytes).map_err(|_| JwtError::InvalidJwtClaims)?;

    Ok(claims)
  }

  /// The decoded signature bytes.
  pub fn signature(&self) -> Result<Vec<u8>, JwtError> {
    let parts: Vec<&str> = self.token.split('.').collect();
    if parts.len() != 3 {
      return Err(JwtError::InvalidJwtFormat);
    }

    let signature = URL_SAFE_NO_PAD
      .decode(parts[2])
      .map_err(|_| JwtError::InvalidJwtSignature)?;

    Ok(signature)
  }

  /// Checks `exp`, when present, against the clock; worthless without a verified signature.
  pub fn validate_expiration(&self) -> Result<(), JwtError> {
    let claims = self.claims()?;

    if let Some(exp) = claims.get("exp").and_then(serde_json::Value::as_u64) {
      let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(std::time::Duration::ZERO)
        .as_secs();

      if exp < now {
        return Err(JwtError::TokenExpired);
      }
    }

    Ok(())
  }

  /// Checks `nbf`, when present, against the clock; worthless without a verified signature.
  pub fn validate_not_before(&self) -> Result<(), JwtError> {
    let claims = self.claims()?;

    if let Some(nbf) = claims.get("nbf").and_then(serde_json::Value::as_u64) {
      let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(std::time::Duration::ZERO)
        .as_secs();

      if nbf > now {
        return Err(JwtError::TokenNotYetValid);
      }
    }

    Ok(())
  }
}

impl<T> JwtClaimsUnverified<T>
where
  T: DeserializeOwned,
{
  fn extract_from_headers(headers: &http::HeaderMap) -> Result<Self, JwtError> {
    let jwt = Jwt::extract_from_headers(headers)?;

    jwt.validate_format()?;

    let claims_json = jwt.claims()?;
    let claims: T = serde_json::from_value(claims_json)
      .map_err(|e| JwtError::ClaimsDeserializationError(e.to_string()))?;

    Ok(JwtClaimsUnverified(claims))
  }
}

impl<'a> FromRequest<'a> for Jwt {
  type Error = JwtError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl Future<Output = Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from_headers(req.headers()))
  }
}

impl<'a> FromRequestParts<'a> for Jwt {
  type Error = JwtError;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl Future<Output = Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from_headers(&parts.headers))
  }
}

impl<'a, T> FromRequest<'a> for JwtClaimsUnverified<T>
where
  T: DeserializeOwned + Send + 'a,
{
  type Error = JwtError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl Future<Output = Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from_headers(req.headers()))
  }
}

impl<'a, T> FromRequestParts<'a> for JwtClaimsUnverified<T>
where
  T: DeserializeOwned + Send + 'a,
{
  type Error = JwtError;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl Future<Output = Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from_headers(&parts.headers))
  }
}
