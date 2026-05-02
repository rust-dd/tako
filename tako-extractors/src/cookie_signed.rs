//! Signed cookie extraction and management for HTTP requests.
//!
//! This module provides the [`CookieSigned`](crate::extractors::cookie_signed::CookieSigned) extractor that manages HMAC-signed cookies
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

use cookie::Cookie;
use cookie::CookieJar;
use cookie::Key;
use http::HeaderMap;
use http::StatusCode;
use http::header::COOKIE;
use http::request::Parts;
use tako_core::extractors::FromRequest;
use tako_core::extractors::FromRequestParts;
use tako_core::responder::Responder;
use tako_core::types::Request;

/// Key ring for rotation-aware cookie signing/verification.
///
/// `active` is used to sign new cookies; `previous` keys are tried for
/// verification only, letting old cookies remain valid through a rotation.
/// Each key carries a string `kid` so callers can log which key admitted a
/// given cookie when [`KeyRing::sign_jar`] / [`CookieSigned::get_with_kid`] is used.
#[derive(Clone)]
pub struct KeyRing {
  pub(crate) active_kid: String,
  pub(crate) active: Key,
  pub(crate) previous: Vec<(String, Key)>,
}

impl KeyRing {
  /// Build a key ring with a single active key.
  pub fn new(active_kid: impl Into<String>, active: Key) -> Self {
    Self {
      active_kid: active_kid.into(),
      active,
      previous: Vec::new(),
    }
  }

  /// Add a previous key. Verification tries the active key first, then each
  /// previous key in insertion order.
  pub fn with_previous(mut self, kid: impl Into<String>, key: Key) -> Self {
    self.previous.push((kid.into(), key));
    self
  }

  /// Borrow the active key.
  pub fn active(&self) -> &Key {
    &self.active
  }

  /// The active key id.
  pub fn active_kid(&self) -> &str {
    &self.active_kid
  }
}

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
  /// Optional rotation ring; when present, verification tries every key.
  ring: Option<KeyRing>,
}

/// Error type for signed cookie extraction.
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
  fn into_response(self) -> tako_core::types::Response {
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
        format!("Failed to verify signed cookie: {err}"),
      )
        .into_response(),
      CookieSignedError::InvalidCookieFormat => {
        (StatusCode::BAD_REQUEST, "Invalid cookie format in request").into_response()
      }
      CookieSignedError::InvalidSignature(cookie_name) => (
        StatusCode::BAD_REQUEST,
        format!("Invalid signature for cookie: {cookie_name}"),
      )
        .into_response(),
    }
  }
}

impl CookieSigned {
  /// Creates a new `CookieSigned` instance with the given master key.
  pub fn new(key: Key) -> Self {
    Self {
      jar: CookieJar::new(),
      key,
      ring: None,
    }
  }

  /// Creates a new `CookieSigned` driven by a key ring (rotation-aware).
  pub fn with_ring(ring: KeyRing) -> Self {
    let key = ring.active.clone();
    Self {
      jar: CookieJar::new(),
      key,
      ring: Some(ring),
    }
  }

  /// Creates a `CookieSigned` instance from HTTP headers and a master key.
  pub fn from_headers(headers: &HeaderMap, key: Key) -> Self {
    let mut jar = CookieJar::new();

    if let Some(cookie_header) = headers.get(COOKIE).and_then(|v| v.to_str().ok()) {
      for cookie_str in cookie_header.split(';') {
        if let Ok(cookie) = Cookie::parse(cookie_str.trim()) {
          jar.add_original(cookie.into_owned());
        }
      }
    }

    Self {
      jar,
      key,
      ring: None,
    }
  }

  /// Creates a `CookieSigned` instance from HTTP headers and a key ring.
  pub fn from_headers_with_ring(headers: &HeaderMap, ring: KeyRing) -> Self {
    let mut signed = Self::from_headers(headers, ring.active.clone());
    signed.ring = Some(ring);
    signed
  }

  /// Retrieves a signed cookie, returning the kid that admitted it (if any).
  ///
  /// Tries the active key first, then each previous key in the ring in order.
  /// Returns `(cookie, kid)` on success; `None` if no key in the ring can verify.
  pub fn get_with_kid(&self, name: &str) -> Option<(Cookie<'static>, String)> {
    if let Some(c) = self.jar.signed(&self.key).get(name) {
      let kid = self
        .ring
        .as_ref()
        .map(|r| r.active_kid.clone())
        .unwrap_or_else(|| "default".to_string());
      return Some((c, kid));
    }
    if let Some(ring) = self.ring.as_ref() {
      for (kid, key) in &ring.previous {
        if let Some(c) = self.jar.signed(key).get(name) {
          return Some((c, kid.clone()));
        }
      }
    }
    None
  }

  /// Adds a signed cookie to the jar.
  pub fn add(&mut self, cookie: Cookie<'static>) {
    self.jar.signed_mut(&self.key).add(cookie);
  }

  /// Removes a signed cookie from the jar by its name.
  pub fn remove(&mut self, name: &str) {
    self
      .jar
      .signed_mut(&self.key)
      .remove(Cookie::from(name.to_owned()));
  }

  /// Retrieves and verifies a signed cookie from the jar by its name.
  ///
  /// When a [`KeyRing`] is configured (`with_ring` / `from_headers_with_ring`),
  /// every previous key is tried after the active key fails.
  pub fn get(&self, name: &str) -> Option<Cookie<'static>> {
    if let Some(c) = self.jar.signed(&self.key).get(name) {
      return Some(c);
    }
    if let Some(ring) = self.ring.as_ref() {
      for (_kid, key) in &ring.previous {
        if let Some(c) = self.jar.signed(key).get(name) {
          return Some(c);
        }
      }
    }
    None
  }

  /// Gets the inner `CookieJar` for advanced operations.
  pub fn inner(&self) -> &CookieJar {
    &self.jar
  }

  /// Gets a mutable reference to the inner `CookieJar` for advanced operations.
  pub fn inner_mut(&mut self) -> &mut CookieJar {
    &mut self.jar
  }

  /// Verifies if a cookie with the given name exists and has a valid signature.
  pub fn verify(&self, name: &str) -> bool {
    self.get(name).is_some()
  }

  /// Gets the value of a signed cookie after verification.
  pub fn get_value(&self, name: &str) -> Option<String> {
    self.get(name).map(|cookie| cookie.value().to_string())
  }

  /// Checks if a signed cookie with the given name exists and has a valid signature.
  pub fn contains(&self, name: &str) -> bool {
    self.get(name).is_some()
  }

  /// Gets the key used for signed cookie operations.
  pub fn key(&self) -> &Key {
    &self.key
  }

  /// Extracts signed cookies from a request, preferring a [`KeyRing`] over a
  /// single [`Key`] when both are present in extensions.
  fn extract_from_request(req: &Request) -> Result<Self, CookieSignedError> {
    if let Some(ring) = req.extensions().get::<KeyRing>().cloned() {
      return Ok(Self::from_headers_with_ring(req.headers(), ring));
    }
    let key = req
      .extensions()
      .get::<Key>()
      .ok_or(CookieSignedError::MissingKey)?
      .clone();
    Ok(Self::from_headers(req.headers(), key))
  }

  /// Same as [`Self::extract_from_request`] but for `Parts`.
  fn extract_from_parts(parts: &Parts) -> Result<Self, CookieSignedError> {
    if let Some(ring) = parts.extensions.get::<KeyRing>().cloned() {
      return Ok(Self::from_headers_with_ring(&parts.headers, ring));
    }
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

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from_request(req))
  }
}

impl<'a> FromRequestParts<'a> for CookieSigned {
  type Error = CookieSignedError;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from_parts(parts))
  }
}
