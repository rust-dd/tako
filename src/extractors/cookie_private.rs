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
  pub fn new(key: Key) -> Self {
    Self {
      jar: CookieJar::new(),
      key,
    }
  }

  /// Creates a `CookiePrivate` instance from HTTP headers and a master key.
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
  pub fn add(&mut self, cookie: Cookie<'static>) {
    self.jar.private_mut(&self.key).add(cookie);
  }

  /// Removes a private cookie from the jar by its name.
  pub fn remove(&mut self, name: &str) {
    self
      .jar
      .private_mut(&self.key)
      .remove(Cookie::from(name.to_owned()));
  }

  /// Retrieves and decrypts a private cookie from the jar by its name.
  pub fn get(&self, name: &str) -> Option<Cookie<'static>> {
    self.jar.private(&self.key).get(name)
  }

  /// Gets the value of a private cookie after decryption.
  pub fn get_value(&self, name: &str) -> Option<String> {
    self.get(name).map(|cookie| cookie.value().to_string())
  }

  /// Checks if a private cookie with the given name exists and can be decrypted.
  pub fn contains(&self, name: &str) -> bool {
    self.get(name).is_some()
  }

  /// Gets the inner `CookieJar` for advanced operations.
  pub fn inner(&self) -> &CookieJar {
    &self.jar
  }

  /// Gets a mutable reference to the inner `CookieJar` for advanced operations.
  pub fn inner_mut(&mut self) -> &mut CookieJar {
    &mut self.jar
  }

  /// Gets the key used for private cookie operations.
  pub fn key(&self) -> &Key {
    &self.key
  }

  /// Extracts private cookies from a request using a master key from extensions.
  fn extract_from_request(req: &Request) -> Result<Self, CookiePrivateError> {
    let key = req
      .extensions()
      .get::<Key>()
      .ok_or(CookiePrivateError::MissingKey)?
      .clone();

    Ok(Self::from_headers(req.headers(), key))
  }

  /// Extracts private cookies from request parts using a master key from extensions.
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

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    ready(Self::extract_from_request(req))
  }
}

impl<'a> FromRequestParts<'a> for CookiePrivate {
  type Error = CookiePrivateError;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    ready(Self::extract_from_parts(parts))
  }
}
