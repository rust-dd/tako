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
///
/// # Examples
///
/// ```rust
/// use tako::extractors::bearer::Bearer;
/// use tako::extractors::FromRequest;
/// use tako::types::Request;
///
/// async fn jwt_handler(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
///     let bearer = Bearer::from_request(&mut req).await?;
///
///     // Use the token for JWT verification
///     match verify_jwt_token(&bearer.token) {
///         Ok(claims) => Ok(format!("Welcome, user {}!", claims.sub)),
///         Err(_) => Ok("Invalid JWT token".to_string()),
///     }
/// }
///
/// struct Claims {
///     sub: String,
///     exp: u64,
/// }
///
/// fn verify_jwt_token(token: &str) -> Result<Claims, &'static str> {
///     // In production, use a proper JWT library
///     if token.starts_with("valid_token") {
///         Ok(Claims { sub: "user123".to_string(), exp: 1234567890 })
///     } else {
///         Err("Invalid token")
///     }
/// }
/// ```
pub struct Bearer {
    /// Token value extracted from Bearer auth header (without "Bearer " prefix).
    pub token: String,
    /// Complete Bearer token string as received ("Bearer " + token).
    pub with_bearer: String,
}

/// Error types for Bearer token authentication extraction and validation.
///
/// These errors cover the various failure modes when parsing Bearer token
/// Authorization headers, from missing headers to empty tokens. Each error
/// provides specific information to help with debugging authentication issues.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::bearer::{Bearer, BearerAuthError};
/// use tako::responder::Responder;
/// use http::StatusCode;
///
/// async fn handle_auth_error(error: BearerAuthError) -> String {
///     match error {
///         BearerAuthError::MissingAuthHeader => "Please provide Authorization header".to_string(),
///         BearerAuthError::InvalidBearerFormat => "Use Bearer token format".to_string(),
///         BearerAuthError::EmptyToken => "Token cannot be empty".to_string(),
///         _ => "Authentication error".to_string(),
///     }
/// }
/// ```
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
    ///
    /// All errors return 401 Unauthorized status with descriptive messages
    /// to help clients understand Bearer token requirements and failures.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::bearer::BearerAuthError;
    /// use tako::responder::Responder;
    /// use http::StatusCode;
    ///
    /// let error = BearerAuthError::EmptyToken;
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    /// ```
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
    /// Extracts Bearer token from HTTP headers.
    ///
    /// Parses the Authorization header, validates the Bearer authentication scheme,
    /// and extracts the token value. The token must be non-empty after the
    /// "Bearer " prefix. Both the clean token and the full header value are
    /// preserved for different use cases.
    ///
    /// # Errors
    ///
    /// Returns various `BearerAuthError` variants for different failure modes:
    /// - Missing Authorization header
    /// - Invalid header format or encoding
    /// - Non-Bearer authentication scheme
    /// - Empty token value
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::bearer::Bearer;
    /// use http::HeaderMap;
    ///
    /// let mut headers = HeaderMap::new();
    /// headers.insert("authorization", "Bearer abc123xyz".parse().unwrap());
    ///
    /// let result = Bearer::extract_from_headers(&headers);
    /// match result {
    ///     Ok(bearer) => {
    ///         assert_eq!(bearer.token, "abc123xyz");
    ///         assert_eq!(bearer.with_bearer, "Bearer abc123xyz");
    ///     }
    ///     Err(_) => panic!("Should parse valid Bearer token"),
    /// }
    ///
    /// // Test with empty token
    /// let mut empty_headers = HeaderMap::new();
    /// empty_headers.insert("authorization", "Bearer ".parse().unwrap());
    /// assert!(Bearer::extract_from_headers(&empty_headers).is_err());
    /// ```
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

    /// Extracts Bearer token from the complete HTTP request.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::bearer::Bearer;
    /// use tako::extractors::FromRequest;
    /// use tako::types::Request;
    ///
    /// async fn protected_api(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
    ///     let bearer = Bearer::from_request(&mut req).await?;
    ///
    ///     // Validate token against your authentication system
    ///     if validate_access_token(&bearer.token).await? {
    ///         Ok("API access granted".to_string())
    ///     } else {
    ///         Ok("Invalid or expired token".to_string())
    ///     }
    /// }
    ///
    /// async fn validate_access_token(token: &str) -> Result<bool, Box<dyn std::error::Error>> {
    ///     // Implement your token validation logic:
    ///     // - JWT verification with proper libraries
    ///     // - Database lookup for session tokens
    ///     // - OAuth token introspection
    ///     // - Custom token validation logic
    ///     Ok(token.len() > 10) // Simplified example
    /// }
    /// ```
    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_headers(req.headers()))
    }
}

impl<'a> FromRequestParts<'a> for Bearer {
    type Error = BearerAuthError;

    /// Extracts Bearer token from HTTP request parts.
    ///
    /// This is more efficient when you only need headers and don't require
    /// access to the request body. Particularly useful in authentication
    /// middleware that runs before request body processing.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::bearer::Bearer;
    /// use tako::extractors::FromRequestParts;
    /// use http::request::Parts;
    ///
    /// async fn auth_middleware(bearer: Bearer) -> String {
    ///     format!("Authenticated with token: {}...", &bearer.token[..8])
    /// }
    ///
    /// // Combine with other header-based extractors
    /// async fn api_with_auth(
    ///     bearer: Bearer,
    ///     user_agent: Option<String>,
    /// ) -> String {
    ///     format!("Token: {}..., UA: {:?}", &bearer.token[..8], user_agent)
    /// }
    ///
    /// // JWT claims extraction example
    /// async fn jwt_claims_handler(bearer: Bearer) -> Result<String, &'static str> {
    ///     let claims = decode_jwt(&bearer.token)?;
    ///     Ok(format!("Welcome, {}!", claims.username))
    /// }
    ///
    /// struct JwtClaims {
    ///     username: String,
    ///     exp: u64,
    /// }
    ///
    /// fn decode_jwt(token: &str) -> Result<JwtClaims, &'static str> {
    ///     // Use a proper JWT library in production
    ///     if token.starts_with("eyJ") {
    ///         Ok(JwtClaims { username: "user".to_string(), exp: 1234567890 })
    ///     } else {
    ///         Err("Invalid JWT")
    ///     }
    /// }
    /// ```
    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_headers(&parts.headers))
    }
}
