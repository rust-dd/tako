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
///
/// # Examples
///
/// ```rust
/// use tako::extractors::basic::Basic;
/// use tako::extractors::FromRequest;
/// use tako::types::Request;
/// use tako::body::TakoBody;
///
/// async fn login_handler(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
///     let auth = Basic::from_request(&mut req).await?;
///
///     println!("Login attempt: user={}", auth.username);
///     // Note: Never log passwords in production!
///
///     // Perform authentication logic here
///     if validate_credentials(&auth.username, &auth.password) {
///         Ok("Authentication successful".to_string())
///     } else {
///         Ok("Authentication failed".to_string())
///     }
/// }
///
/// fn validate_credentials(username: &str, password: &str) -> bool {
///     // In production, check against secure storage
///     username == "admin" && password == "secret"
/// }
/// ```
pub struct Basic {
    /// Username extracted from the Basic auth token.
    pub username: String,
    /// Password extracted from the Basic auth token.
    pub password: String,
    /// Raw Basic auth token as received in the Authorization header.
    pub raw: String,
}

/// Error types for Basic authentication extraction and validation.
///
/// These errors cover the various failure modes when parsing Basic authentication
/// headers, from missing headers to malformed credential formats. Each error
/// maps to appropriate HTTP status codes and user-friendly error messages.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::basic::{Basic, BasicAuthError};
/// use tako::responder::Responder;
/// use http::StatusCode;
///
/// // Handle authentication errors in middleware
/// async fn auth_error_handler(error: BasicAuthError) -> String {
///     match error {
///         BasicAuthError::MissingAuthHeader => "Please provide credentials".to_string(),
///         BasicAuthError::InvalidBasicFormat => "Use Basic authentication".to_string(),
///         _ => "Authentication error".to_string(),
///     }
/// }
/// ```
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
    ///
    /// All errors return 401 Unauthorized status with descriptive messages
    /// to help clients understand authentication requirements and failures.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::basic::BasicAuthError;
    /// use tako::responder::Responder;
    /// use http::StatusCode;
    ///
    /// let error = BasicAuthError::MissingAuthHeader;
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    /// ```
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
    /// Extracts Basic authentication credentials from HTTP headers.
    ///
    /// Parses the Authorization header, validates the Basic authentication scheme,
    /// decodes the Base64-encoded credentials, and splits them into username and
    /// password components. The credentials must be in the format "username:password".
    ///
    /// # Errors
    ///
    /// Returns various `BasicAuthError` variants for different failure modes:
    /// - Missing Authorization header
    /// - Invalid header format or encoding
    /// - Malformed credential structure
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::basic::Basic;
    /// use http::HeaderMap;
    ///
    /// let mut headers = HeaderMap::new();
    /// headers.insert("authorization", "Basic YWRtaW46c2VjcmV0".parse().unwrap());
    ///
    /// let result = Basic::extract_from_headers(&headers);
    /// match result {
    ///     Ok(basic) => {
    ///         assert_eq!(basic.username, "admin");
    ///         assert_eq!(basic.password, "secret");
    ///     }
    ///     Err(_) => panic!("Should parse valid credentials"),
    /// }
    /// ```
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

    /// Extracts Basic authentication credentials from the complete HTTP request.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::basic::Basic;
    /// use tako::extractors::FromRequest;
    /// use tako::types::Request;
    ///
    /// async fn protected_endpoint(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
    ///     let auth = Basic::from_request(&mut req).await?;
    ///
    ///     // Validate against your authentication system
    ///     if authenticate_user(&auth.username, &auth.password).await? {
    ///         Ok(format!("Hello, {}!", auth.username))
    ///     } else {
    ///         Ok("Access denied".to_string())
    ///     }
    /// }
    ///
    /// async fn authenticate_user(username: &str, password: &str) -> Result<bool, Box<dyn std::error::Error>> {
    ///     // Implement your authentication logic here
    ///     Ok(username == "admin" && password == "secret")
    /// }
    /// ```
    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_headers(req.headers()))
    }
}

impl<'a> FromRequestParts<'a> for Basic {
    type Error = BasicAuthError;

    /// Extracts Basic authentication credentials from HTTP request parts.
    ///
    /// This is more efficient when you only need headers and don't require
    /// access to the request body. Useful in middleware or when extracting
    /// multiple header-based values.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::basic::Basic;
    /// use tako::extractors::FromRequestParts;
    /// use http::request::Parts;
    ///
    /// async fn auth_middleware(basic: Basic) -> String {
    ///     format!("Authenticated as: {}", basic.username)
    /// }
    ///
    /// // Can be used in combination with other extractors
    /// async fn handler_with_auth(basic: Basic, path: tako::extractors::path::Path<'_>) -> String {
    ///     format!("User {} accessing {}", basic.username, path.0)
    /// }
    /// ```
    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_headers(&parts.headers))
    }
}
