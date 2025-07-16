//! Signed cookie extraction and management for HTTP requests.
//!
//! This module provides the [`CookieSigned`] extractor that manages HMAC-signed cookies
//! using a master key. Signed cookies use HMAC (Hash-based Message Authentication Code)
//! to ensure that cookie values haven't been tampered with, while keeping the content
//! readable. This provides integrity protection without confidentiality.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::cookie_signed::CookieSigned;
//! use cookie::{Cookie, Key};
//!
//! async fn handle_signed_cookies(mut signed: CookieSigned) {
//!     // Add a signed cookie
//!     signed.add(Cookie::new("user_id", "12345"));
//!
//!     // Retrieve and verify a cookie
//!     if let Some(user_id) = signed.get_value("user_id") {
//!         println!("User ID: {}", user_id);
//!     }
//!
//!     // Check if a cookie exists and has valid signature
//!     if signed.verify("session_token") {
//!         println!("Valid session found");
//!     }
//! }
//! ```

use cookie::{Cookie, CookieJar, Key};
use http::{HeaderMap, StatusCode, header::COOKIE, request::Parts};
use std::future::ready;

use crate::{
    extractors::{FromRequest, FromRequestParts},
    responder::Responder,
    types::Request,
};

/// A wrapper that provides methods for managing HMAC-signed cookies in HTTP requests and responses.
///
/// Signed cookies use HMAC (Hash-based Message Authentication Code) to ensure
/// that cookie values haven't been tampered with, while keeping the content readable.
/// This provides integrity protection without confidentiality - the cookie values
/// can still be read by clients, but any tampering will be detected.
///
/// The extractor automatically verifies signatures when retrieving cookies and signs
/// cookies when adding them to the jar.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::cookie_signed::CookieSigned;
/// use cookie::{Cookie, Key};
///
/// let key = Key::generate();
/// let mut signed = CookieSigned::new(key);
///
/// // Add a signed cookie
/// signed.add(Cookie::new("username", "alice"));
///
/// // Retrieve the verified value
/// if let Some(username_cookie) = signed.get("username") {
///     assert_eq!(username_cookie.value(), "alice");
/// }
/// ```
pub struct CookieSigned {
    jar: CookieJar,
    key: Key,
}

/// Error type for signed cookie extraction.
///
/// Represents various failure modes that can occur when extracting or processing
/// signed cookies, including missing keys, invalid keys, and signature verification failures.
#[derive(Debug)]
pub enum CookieSignedError {
    /// Signed cookie master key not found in request extensions.
    MissingKey,
    /// Invalid signed cookie master key.
    InvalidKey,
    /// Failed to verify signed cookie with the specified error message.
    VerificationFailed(String),
    /// Invalid cookie format in request.
    InvalidCookieFormat,
    /// Invalid signature for the specified cookie name.
    InvalidSignature(String),
}

impl Responder for CookieSignedError {
    /// Converts the error into an HTTP response.
    ///
    /// Maps signed cookie errors to appropriate HTTP status codes with descriptive
    /// error messages. Server-side errors (missing/invalid keys) result in 500 status,
    /// while client-side errors (bad cookies, invalid signatures) result in 400 status.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_signed::CookieSignedError;
    /// use tako::responder::Responder;
    /// use http::StatusCode;
    ///
    /// let error = CookieSignedError::MissingKey;
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    ///
    /// let error = CookieSignedError::InvalidSignature("session".to_string());
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    /// ```
    fn into_response(self) -> crate::types::Response {
        match self {
            CookieSignedError::MissingKey => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Signed cookie master key not found in request extensions",
            )
                .into_response(),
            CookieSignedError::InvalidKey => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Invalid signed cookie master key",
            )
                .into_response(),
            CookieSignedError::VerificationFailed(err) => (
                StatusCode::BAD_REQUEST,
                format!("Failed to verify signed cookie: {}", err),
            )
                .into_response(),
            CookieSignedError::InvalidCookieFormat => {
                (StatusCode::BAD_REQUEST, "Invalid cookie format in request").into_response()
            }
            CookieSignedError::InvalidSignature(cookie_name) => (
                StatusCode::BAD_REQUEST,
                format!("Invalid signature for cookie: {}", cookie_name),
            )
                .into_response(),
        }
    }
}

impl CookieSigned {
    /// Creates a new `CookieSigned` instance with the given master key.
    ///
    /// # Arguments
    ///
    /// * `key` - The master key used for signing and verifying cookies
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_signed::CookieSigned;
    /// use cookie::Key;
    ///
    /// let key = Key::generate();
    /// let signed = CookieSigned::new(key);
    /// ```
    pub fn new(key: Key) -> Self {
        Self {
            jar: CookieJar::new(),
            key,
        }
    }

    /// Creates a `CookieSigned` instance from HTTP headers and a master key.
    ///
    /// Parses the `Cookie` header and populates the jar with all cookies found.
    /// The cookies remain signed until accessed through the signed cookie methods.
    ///
    /// # Arguments
    ///
    /// * `headers` - HTTP headers containing the `Cookie` header to parse
    /// * `key` - The master key used for verifying cookie signatures
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_signed::CookieSigned;
    /// use cookie::Key;
    /// use http::HeaderMap;
    ///
    /// let mut headers = HeaderMap::new();
    /// headers.insert(
    ///     http::header::COOKIE,
    ///     "session=signed_value; theme=signed_theme".parse().unwrap()
    /// );
    ///
    /// let key = Key::generate();
    /// let signed = CookieSigned::from_headers(&headers, key);
    /// ```
    pub fn from_headers(headers: &HeaderMap, key: Key) -> Self {
        let mut jar = CookieJar::new();

        if let Some(cookie_header) = headers.get(COOKIE).and_then(|v| v.to_str().ok()) {
            for cookie_str in cookie_header.split(';') {
                if let Ok(cookie) = Cookie::parse(cookie_str.trim()) {
                    jar.add_original(cookie.into_owned());
                }
            }
        }

        Self { jar, key }
    }

    /// Adds a signed cookie to the jar.
    ///
    /// The cookie will be signed with HMAC when serialized using the master key.
    /// The original cookie value is preserved but a signature is appended to ensure integrity.
    ///
    /// # Arguments
    ///
    /// * `cookie` - The cookie to add and sign
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_signed::CookieSigned;
    /// use cookie::{Cookie, Key};
    ///
    /// let key = Key::generate();
    /// let mut signed = CookieSigned::new(key);
    ///
    /// signed.add(Cookie::new("user_session", "session_data"));
    ///
    /// // The cookie is now signed in the jar
    /// assert!(signed.verify("user_session"));
    /// ```
    pub fn add(&mut self, cookie: Cookie<'static>) {
        self.jar.signed_mut(&self.key).add(cookie);
    }

    /// Removes a signed cookie from the jar by its name.
    ///
    /// If a cookie with the specified name exists, it will be removed from the jar.
    /// The removal works regardless of whether the cookie has a valid signature.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the cookie to remove
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_signed::CookieSigned;
    /// use cookie::{Cookie, Key};
    ///
    /// let key = Key::generate();
    /// let mut signed = CookieSigned::new(key);
    ///
    /// signed.add(Cookie::new("temp_data", "temporary"));
    /// signed.remove("temp_data");
    ///
    /// assert!(!signed.contains("temp_data"));
    /// ```
    pub fn remove(&mut self, name: &str) {
        self.jar
            .signed_mut(&self.key)
            .remove(Cookie::from(name.to_owned()));
    }

    /// Retrieves and verifies a signed cookie from the jar by its name.
    ///
    /// Attempts to find and verify the specified cookie. Returns the cookie
    /// if it exists and has a valid signature.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the cookie to retrieve
    ///
    /// # Returns
    ///
    /// An `Option` containing the verified cookie if it exists and has a valid signature,
    /// or `None` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_signed::CookieSigned;
    /// use cookie::{Cookie, Key};
    ///
    /// let key = Key::generate();
    /// let mut signed = CookieSigned::new(key);
    ///
    /// signed.add(Cookie::new("user_id", "12345"));
    ///
    /// if let Some(cookie) = signed.get("user_id") {
    ///     assert_eq!(cookie.value(), "12345");
    /// }
    ///
    /// assert!(signed.get("nonexistent").is_none());
    /// ```
    pub fn get(&self, name: &str) -> Option<Cookie<'static>> {
        self.jar.signed(&self.key).get(name)
    }

    /// Gets the inner `CookieJar` for advanced operations.
    ///
    /// Provides access to the underlying cookie jar for operations that are not
    /// directly supported by the signed cookie interface.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_signed::CookieSigned;
    /// use cookie::Key;
    ///
    /// let key = Key::generate();
    /// let signed = CookieSigned::new(key);
    ///
    /// let jar = signed.inner();
    /// // Use jar for advanced operations
    /// ```
    pub fn inner(&self) -> &CookieJar {
        &self.jar
    }

    /// Gets a mutable reference to the inner `CookieJar` for advanced operations.
    ///
    /// Provides mutable access to the underlying cookie jar for operations that
    /// require modification and are not directly supported by the signed cookie interface.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_signed::CookieSigned;
    /// use cookie::Key;
    ///
    /// let key = Key::generate();
    /// let mut signed = CookieSigned::new(key);
    ///
    /// let jar = signed.inner_mut();
    /// // Use jar for advanced mutable operations
    /// ```
    pub fn inner_mut(&mut self) -> &mut CookieJar {
        &mut self.jar
    }

    /// Verifies if a cookie with the given name exists and has a valid signature.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the cookie to verify
    ///
    /// # Returns
    ///
    /// `true` if the cookie exists and has a valid signature, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_signed::CookieSigned;
    /// use cookie::{Cookie, Key};
    ///
    /// let key = Key::generate();
    /// let mut signed = CookieSigned::new(key);
    ///
    /// signed.add(Cookie::new("token", "abc123"));
    ///
    /// assert!(signed.verify("token"));
    /// assert!(!signed.verify("missing"));
    /// ```
    pub fn verify(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Gets the value of a signed cookie after verification.
    ///
    /// Convenience method that retrieves a cookie and extracts its value in one step.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the cookie whose value to retrieve
    ///
    /// # Returns
    ///
    /// An `Option` containing the cookie value if the cookie exists and has a valid signature.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_signed::CookieSigned;
    /// use cookie::{Cookie, Key};
    ///
    /// let key = Key::generate();
    /// let mut signed = CookieSigned::new(key);
    ///
    /// signed.add(Cookie::new("username", "alice"));
    ///
    /// if let Some(username) = signed.get_value("username") {
    ///     assert_eq!(username, "alice");
    /// }
    /// ```
    pub fn get_value(&self, name: &str) -> Option<String> {
        self.get(name).map(|cookie| cookie.value().to_string())
    }

    /// Checks if a signed cookie with the given name exists and has a valid signature.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the cookie to check
    ///
    /// # Returns
    ///
    /// `true` if the cookie exists and has a valid signature, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_signed::CookieSigned;
    /// use cookie::{Cookie, Key};
    ///
    /// let key = Key::generate();
    /// let mut signed = CookieSigned::new(key);
    ///
    /// signed.add(Cookie::new("token", "abc123"));
    ///
    /// assert!(signed.contains("token"));
    /// assert!(!signed.contains("missing"));
    /// ```
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Gets the key used for signed cookie operations.
    ///
    /// Returns a reference to the signing key used for signing and verifying
    /// cookies in this jar.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_signed::CookieSigned;
    /// use cookie::Key;
    ///
    /// let original_key = Key::generate();
    /// let signed = CookieSigned::new(original_key.clone());
    ///
    /// let key_ref = signed.key();
    /// // key_ref can be used to verify it's the same key
    /// ```
    pub fn key(&self) -> &Key {
        &self.key
    }

    /// Extracts signed cookies from a request using a master key from extensions.
    ///
    /// # Arguments
    ///
    /// * `req` - The HTTP request containing the master key in extensions
    ///
    /// # Errors
    ///
    /// Returns `CookieSignedError::MissingKey` if no master key is found in extensions.
    fn extract_from_request(req: &Request) -> Result<Self, CookieSignedError> {
        let key = req
            .extensions()
            .get::<Key>()
            .ok_or(CookieSignedError::MissingKey)?
            .clone();

        Ok(Self::from_headers(req.headers(), key))
    }

    /// Extracts signed cookies from request parts using a master key from extensions.
    ///
    /// # Arguments
    ///
    /// * `parts` - The HTTP request parts containing the master key in extensions
    ///
    /// # Errors
    ///
    /// Returns `CookieSignedError::MissingKey` if no master key is found in extensions.
    fn extract_from_parts(parts: &Parts) -> Result<Self, CookieSignedError> {
        let key = parts
            .extensions
            .get::<Key>()
            .ok_or(CookieSignedError::MissingKey)?
            .clone();

        Ok(Self::from_headers(&parts.headers, key))
    }
}

impl<'a> FromRequest<'a> for CookieSigned {
    type Error = CookieSignedError;

    /// Extracts `CookieSigned` from an HTTP request.
    ///
    /// The master key must be placed in the request extensions before calling
    /// this extractor. This is typically done by middleware that sets up the
    /// cryptographic context for the application.
    ///
    /// # Errors
    ///
    /// Returns `CookieSignedError::MissingKey` if no master key is found
    /// in the request extensions.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::{FromRequest, cookie_signed::CookieSigned};
    /// use tako::types::Request;
    ///
    /// async fn handler(mut req: Request) -> Result<(), Box<dyn std::error::Error>> {
    ///     let signed = CookieSigned::from_request(&mut req).await?;
    ///
    ///     if let Some(user_id) = signed.get_value("user_id") {
    ///         println!("User ID: {}", user_id);
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_request(req))
    }
}

impl<'a> FromRequestParts<'a> for CookieSigned {
    type Error = CookieSignedError;

    /// Extracts `CookieSigned` from HTTP request parts.
    ///
    /// The master key must be placed in the request extensions before calling
    /// this extractor. This is typically done by middleware that sets up the
    /// cryptographic context for the application.
    ///
    /// # Errors
    ///
    /// Returns `CookieSignedError::MissingKey` if no master key is found
    /// in the request extensions.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::{FromRequestParts, cookie_signed::CookieSigned};
    /// use http::request::Parts;
    ///
    /// async fn handler(mut parts: Parts) -> Result<(), Box<dyn std::error::Error>> {
    ///     let signed = CookieSigned::from_request_parts(&mut parts).await?;
    ///
    ///     // Check for authentication cookie
    ///     if signed.verify("auth_token") {
    ///         println!("User is authenticated");
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_parts(parts))
    }
}
