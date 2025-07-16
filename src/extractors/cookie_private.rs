//! Private cookie extraction and management for HTTP requests.
//!
//! This module provides the [`CookiePrivate`] extractor that manages encrypted cookies
//! using a master key. Private cookies are encrypted to ensure that cookie values
//! cannot be read or tampered with by clients, providing a secure way to store
//! sensitive data in client-side cookies.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::cookie_private::CookiePrivate;
//! use cookie::{Cookie, Key};
//!
//! async fn handle_private_cookies(mut private: CookiePrivate) {
//!     // Add an encrypted cookie
//!     private.add(Cookie::new("user_id", "12345"));
//!
//!     // Retrieve and decrypt a cookie
//!     if let Some(user_id) = private.get_value("user_id") {
//!         println!("User ID: {}", user_id);
//!     }
//!
//!     // Check if a cookie exists and can be decrypted
//!     if private.contains("session_token") {
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

/// A wrapper that provides methods for managing encrypted cookies in HTTP requests and responses.
///
/// Private cookies are encrypted using a master key, ensuring that cookie
/// values cannot be read or tampered with by clients. This provides a secure
/// way to store sensitive information in cookies while maintaining compatibility
/// with standard HTTP cookie mechanisms.
///
/// The extractor automatically decrypts cookies when retrieving them and encrypts
/// cookies when adding them to the jar.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::cookie_private::CookiePrivate;
/// use cookie::{Cookie, Key};
///
/// let key = Key::generate();
/// let mut private = CookiePrivate::new(key);
///
/// // Add an encrypted cookie
/// private.add(Cookie::new("secret", "sensitive_data"));
///
/// // Retrieve the decrypted value
/// if let Some(secret_cookie) = private.get("secret") {
///     assert_eq!(secret_cookie.value(), "sensitive_data");
/// }
/// ```
pub struct CookiePrivate {
    jar: CookieJar,
    key: Key,
}

/// Error type for private cookie extraction.
///
/// Represents various failure modes that can occur when extracting or processing
/// private cookies, including missing keys, invalid keys, and decryption failures.
#[derive(Debug)]
pub enum CookiePrivateError {
    /// Private cookie master key not found in request extensions.
    MissingKey,
    /// Invalid private cookie master key.
    InvalidKey,
    /// Failed to decrypt private cookie with the specified error message.
    DecryptionFailed(String),
    /// Invalid cookie format in request.
    InvalidCookieFormat,
}

impl Responder for CookiePrivateError {
    /// Converts the error into an HTTP response.
    ///
    /// Maps private cookie errors to appropriate HTTP status codes with descriptive
    /// error messages. Server-side errors (missing/invalid keys) result in 500 status,
    /// while client-side errors (bad cookies) result in 400 status.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_private::CookiePrivateError;
    /// use tako::responder::Responder;
    /// use http::StatusCode;
    ///
    /// let error = CookiePrivateError::MissingKey;
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    ///
    /// let error = CookiePrivateError::InvalidCookieFormat;
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    /// ```
    fn into_response(self) -> crate::types::Response {
        match self {
            CookiePrivateError::MissingKey => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Private cookie master key not found in request extensions",
            )
                .into_response(),
            CookiePrivateError::InvalidKey => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Invalid private cookie master key",
            )
                .into_response(),
            CookiePrivateError::DecryptionFailed(err) => (
                StatusCode::BAD_REQUEST,
                format!("Failed to decrypt private cookie: {}", err),
            )
                .into_response(),
            CookiePrivateError::InvalidCookieFormat => {
                (StatusCode::BAD_REQUEST, "Invalid cookie format in request").into_response()
            }
        }
    }
}

impl CookiePrivate {
    /// Creates a new `CookiePrivate` instance with the given master key.
    ///
    /// # Arguments
    ///
    /// * `key` - The master key used for encrypting and decrypting cookies
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_private::CookiePrivate;
    /// use cookie::Key;
    ///
    /// let key = Key::generate();
    /// let private = CookiePrivate::new(key);
    /// ```
    pub fn new(key: Key) -> Self {
        Self {
            jar: CookieJar::new(),
            key,
        }
    }

    /// Creates a `CookiePrivate` instance from HTTP headers and a master key.
    ///
    /// Parses the `Cookie` header and populates the jar with all cookies found.
    /// The cookies remain encrypted until accessed through the private cookie methods.
    ///
    /// # Arguments
    ///
    /// * `headers` - HTTP headers containing the `Cookie` header to parse
    /// * `key` - The master key used for decrypting cookies
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_private::CookiePrivate;
    /// use cookie::Key;
    /// use http::HeaderMap;
    ///
    /// let mut headers = HeaderMap::new();
    /// headers.insert(
    ///     http::header::COOKIE,
    ///     "session=encrypted_value; theme=encrypted_theme".parse().unwrap()
    /// );
    ///
    /// let key = Key::generate();
    /// let private = CookiePrivate::from_headers(&headers, key);
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

    /// Adds a private cookie to the jar.
    ///
    /// The cookie will be encrypted when serialized using the master key.
    /// The original cookie value is encrypted and replaced with the encrypted version.
    ///
    /// # Arguments
    ///
    /// * `cookie` - The cookie to add and encrypt
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_private::CookiePrivate;
    /// use cookie::{Cookie, Key};
    ///
    /// let key = Key::generate();
    /// let mut private = CookiePrivate::new(key);
    ///
    /// private.add(Cookie::new("user_session", "secret_session_data"));
    ///
    /// // The cookie is now encrypted in the jar
    /// assert!(private.contains("user_session"));
    /// ```
    pub fn add(&mut self, cookie: Cookie<'static>) {
        self.jar.private_mut(&self.key).add(cookie);
    }

    /// Removes a private cookie from the jar by its name.
    ///
    /// If a cookie with the specified name exists, it will be removed from the jar.
    /// The removal works regardless of whether the cookie can be successfully decrypted.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the cookie to remove
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_private::CookiePrivate;
    /// use cookie::{Cookie, Key};
    ///
    /// let key = Key::generate();
    /// let mut private = CookiePrivate::new(key);
    ///
    /// private.add(Cookie::new("temp_data", "temporary"));
    /// private.remove("temp_data");
    ///
    /// assert!(!private.contains("temp_data"));
    /// ```
    pub fn remove(&mut self, name: &str) {
        self.jar
            .private_mut(&self.key)
            .remove(Cookie::from(name.to_owned()));
    }

    /// Retrieves and decrypts a private cookie from the jar by its name.
    ///
    /// Attempts to find and decrypt the specified cookie. Returns the decrypted
    /// cookie if it exists and can be successfully decrypted.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the cookie to retrieve
    ///
    /// # Returns
    ///
    /// An `Option` containing the decrypted cookie if it exists and can be
    /// successfully decrypted, or `None` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_private::CookiePrivate;
    /// use cookie::{Cookie, Key};
    ///
    /// let key = Key::generate();
    /// let mut private = CookiePrivate::new(key);
    ///
    /// private.add(Cookie::new("user_id", "12345"));
    ///
    /// if let Some(cookie) = private.get("user_id") {
    ///     assert_eq!(cookie.value(), "12345");
    /// }
    ///
    /// assert!(private.get("nonexistent").is_none());
    /// ```
    pub fn get(&self, name: &str) -> Option<Cookie<'static>> {
        self.jar.private(&self.key).get(name)
    }

    /// Gets the value of a private cookie after decryption.
    ///
    /// Convenience method that retrieves a cookie and extracts its value in one step.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the cookie whose value to retrieve
    ///
    /// # Returns
    ///
    /// An `Option` containing the cookie value if the cookie exists and can be decrypted.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_private::CookiePrivate;
    /// use cookie::{Cookie, Key};
    ///
    /// let key = Key::generate();
    /// let mut private = CookiePrivate::new(key);
    ///
    /// private.add(Cookie::new("username", "alice"));
    ///
    /// if let Some(username) = private.get_value("username") {
    ///     assert_eq!(username, "alice");
    /// }
    /// ```
    pub fn get_value(&self, name: &str) -> Option<String> {
        self.get(name).map(|cookie| cookie.value().to_string())
    }

    /// Checks if a private cookie with the given name exists and can be decrypted.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the cookie to check
    ///
    /// # Returns
    ///
    /// `true` if the cookie exists and can be decrypted, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_private::CookiePrivate;
    /// use cookie::{Cookie, Key};
    ///
    /// let key = Key::generate();
    /// let mut private = CookiePrivate::new(key);
    ///
    /// private.add(Cookie::new("token", "abc123"));
    ///
    /// assert!(private.contains("token"));
    /// assert!(!private.contains("missing"));
    /// ```
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Gets the inner `CookieJar` for advanced operations.
    ///
    /// Provides access to the underlying cookie jar for operations that are not
    /// directly supported by the private cookie interface.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_private::CookiePrivate;
    /// use cookie::Key;
    ///
    /// let key = Key::generate();
    /// let private = CookiePrivate::new(key);
    ///
    /// let jar = private.inner();
    /// // Use jar for advanced operations
    /// ```
    pub fn inner(&self) -> &CookieJar {
        &self.jar
    }

    /// Gets a mutable reference to the inner `CookieJar` for advanced operations.
    ///
    /// Provides mutable access to the underlying cookie jar for operations that
    /// require modification and are not directly supported by the private cookie interface.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_private::CookiePrivate;
    /// use cookie::Key;
    ///
    /// let key = Key::generate();
    /// let mut private = CookiePrivate::new(key);
    ///
    /// let jar = private.inner_mut();
    /// // Use jar for advanced mutable operations
    /// ```
    pub fn inner_mut(&mut self) -> &mut CookieJar {
        &mut self.jar
    }

    /// Gets the key used for private cookie operations.
    ///
    /// Returns a reference to the encryption key used for encrypting and
    /// decrypting cookies in this jar.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_private::CookiePrivate;
    /// use cookie::Key;
    ///
    /// let original_key = Key::generate();
    /// let private = CookiePrivate::new(original_key.clone());
    ///
    /// let key_ref = private.key();
    /// // key_ref can be used to verify it's the same key
    /// ```
    pub fn key(&self) -> &Key {
        &self.key
    }

    /// Extracts private cookies from a request using a master key from extensions.
    ///
    /// # Arguments
    ///
    /// * `req` - The HTTP request containing the master key in extensions
    ///
    /// # Errors
    ///
    /// Returns `CookiePrivateError::MissingKey` if no master key is found in extensions.
    fn extract_from_request(req: &Request) -> Result<Self, CookiePrivateError> {
        let key = req
            .extensions()
            .get::<Key>()
            .ok_or(CookiePrivateError::MissingKey)?
            .clone();

        Ok(Self::from_headers(req.headers(), key))
    }

    /// Extracts private cookies from request parts using a master key from extensions.
    ///
    /// # Arguments
    ///
    /// * `parts` - The HTTP request parts containing the master key in extensions
    ///
    /// # Errors
    ///
    /// Returns `CookiePrivateError::MissingKey` if no master key is found in extensions.
    fn extract_from_parts(parts: &Parts) -> Result<Self, CookiePrivateError> {
        let key = parts
            .extensions
            .get::<Key>()
            .ok_or(CookiePrivateError::MissingKey)?
            .clone();

        Ok(Self::from_headers(&parts.headers, key))
    }
}

impl<'a> FromRequest<'a> for CookiePrivate {
    type Error = CookiePrivateError;

    /// Extracts `CookiePrivate` from an HTTP request.
    ///
    /// The master key must be placed in the request extensions before calling
    /// this extractor. This is typically done by middleware that sets up the
    /// cryptographic context for the application.
    ///
    /// # Errors
    ///
    /// Returns `CookiePrivateError::MissingKey` if no master key is found
    /// in the request extensions.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::{FromRequest, cookie_private::CookiePrivate};
    /// use tako::types::Request;
    ///
    /// async fn handler(mut req: Request) -> Result<(), Box<dyn std::error::Error>> {
    ///     let private = CookiePrivate::from_request(&mut req).await?;
    ///
    ///     if let Some(user_id) = private.get_value("user_id") {
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

impl<'a> FromRequestParts<'a> for CookiePrivate {
    type Error = CookiePrivateError;

    /// Extracts `CookiePrivate` from HTTP request parts.
    ///
    /// The master key must be placed in the request extensions before calling
    /// this extractor. This is typically done by middleware that sets up the
    /// cryptographic context for the application.
    ///
    /// # Errors
    ///
    /// Returns `CookiePrivateError::MissingKey` if no master key is found
    /// in the request extensions.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::{FromRequestParts, cookie_private::CookiePrivate};
    /// use http::request::Parts;
    ///
    /// async fn handler(mut parts: Parts) -> Result<(), Box<dyn std::error::Error>> {
    ///     let private = CookiePrivate::from_request_parts(&mut parts).await?;
    ///
    ///     // Check for authentication cookie
    ///     if private.contains("auth_token") {
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
