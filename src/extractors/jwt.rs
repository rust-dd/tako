//! JWT token extraction and claims parsing from Authorization headers.
//!
//! This module provides extractors for parsing JWT (JSON Web Token) tokens from HTTP
//! Authorization headers and extracting claims into strongly-typed Rust structures.
//! It supports both raw JWT access through [`Jwt`](crate::extractors::jwt::Jwt) and automatic claims deserialization
//! through [`JwtClaims`](crate::extractors::jwt::JwtClaims), with built-in token validation and error handling for malformed
//! or expired tokens.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::jwt::{Jwt, JwtClaims};
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
//! async fn protected_handler(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
//!     let jwt_claims: JwtClaims<UserClaims> = JwtClaims::from_request(&mut req).await?;
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

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use http::{StatusCode, request::Parts};
use serde::de::DeserializeOwned;

use crate::{
  extractors::{FromRequest, FromRequestParts},
  responder::Responder,
  types::Request,
};

/// JWT token extractor that provides access to the raw token string.
#[doc(alias = "jwt")]
pub struct Jwt {
  /// Raw JWT token string extracted from Authorization header.
  pub token: String,
  /// Complete Authorization header value.
  pub header: String,
}

/// JWT claims extractor with automatic deserialization to typed structures.
#[doc(alias = "jwt_claims")]
pub struct JwtClaims<T>(pub T);

/// Error types for JWT extraction and claims parsing.
#[derive(Debug)]
pub enum JwtError {
  /// Authorization header is missing from the request.
  MissingAuthHeader,
  /// Authorization header contains invalid UTF-8 or cannot be parsed.
  InvalidAuthHeader,
  /// Authorization header does not use Bearer authentication scheme.
  InvalidBearerFormat,
  /// JWT token is present but empty.
  EmptyToken,
  /// JWT token format is invalid (not three Base64-encoded parts).
  InvalidJwtFormat,
  /// JWT header section cannot be decoded or parsed.
  InvalidJwtHeader,
  /// JWT claims section cannot be decoded or parsed.
  InvalidJwtClaims,
  /// JWT signature section cannot be decoded.
  InvalidJwtSignature,
  /// JWT claims deserialization failed.
  ClaimsDeserializationError(String),
  /// JWT token has expired.
  TokenExpired,
  /// JWT token is not yet valid.
  TokenNotYetValid,
}

impl Responder for JwtError {
  /// Converts JWT errors into appropriate HTTP responses.
  fn into_response(self) -> crate::types::Response {
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
  /// Extracts JWT token from HTTP headers.
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

  /// Validates basic JWT token format.
  pub fn validate_format(&self) -> Result<(), JwtError> {
    let parts = self.token.split('.').collect::<Vec<&str>>();
    if parts.len() != 3 {
      return Err(JwtError::InvalidJwtFormat);
    }

    // Validate that each part is valid base64
    for part in &parts {
      if part.is_empty() {
        return Err(JwtError::InvalidJwtFormat);
      }
    }

    Ok(())
  }

  /// Extracts the header section of the JWT token.
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

  /// Extracts the claims section of the JWT token.
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

  /// Extracts the signature section of the JWT token.
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

  /// Validates token expiration time.
  pub fn validate_expiration(&self) -> Result<(), JwtError> {
    let claims = self.claims()?;

    if let Some(exp) = claims.get("exp").and_then(|v| v.as_u64()) {
      let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

      if exp < now {
        return Err(JwtError::TokenExpired);
      }
    }

    Ok(())
  }

  /// Validates token not-before time.
  pub fn validate_not_before(&self) -> Result<(), JwtError> {
    let claims = self.claims()?;

    if let Some(nbf) = claims.get("nbf").and_then(|v| v.as_u64()) {
      let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

      if nbf > now {
        return Err(JwtError::TokenNotYetValid);
      }
    }

    Ok(())
  }
}

impl<T> JwtClaims<T>
where
  T: DeserializeOwned,
{
  /// Extracts and deserializes JWT claims from HTTP headers.
  fn extract_from_headers(headers: &http::HeaderMap) -> Result<Self, JwtError> {
    let jwt = Jwt::extract_from_headers(headers)?;

    jwt.validate_format()?;
    jwt.validate_expiration()?;
    jwt.validate_not_before()?;

    let claims_json = jwt.claims()?;
    let claims: T = serde_json::from_value(claims_json)
      .map_err(|e| JwtError::ClaimsDeserializationError(e.to_string()))?;

    Ok(JwtClaims(claims))
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

impl<'a, T> FromRequest<'a> for JwtClaims<T>
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

impl<'a, T> FromRequestParts<'a> for JwtClaims<T>
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
