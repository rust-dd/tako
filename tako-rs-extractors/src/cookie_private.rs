//! Encrypted cookies that clients can neither read nor alter. The extractor takes a
//! `KeyRing`, or else a `cookie::Key`, from the request extensions.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::cookie_private::CookiePrivate;
//! use cookie::{Cookie, Key};
//!
//! async fn handle_private_cookies(mut private: CookiePrivate) -> CookiePrivate {
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
//!     private
//! }
//! ```

use cookie::Cookie;
use cookie::CookieJar;
use cookie::Key;
use http::HeaderMap;
use http::StatusCode;
use http::request::Parts;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

/// A cookie jar that encrypts on write and decrypts on read. Return it, or use
/// [`Self::with_response`], to send its changes as `Set-Cookie` headers.
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
  ring: Option<crate::cookie_signed::KeyRing>,
}

/// Rejection of [`CookiePrivate`].
#[derive(Debug)]
pub enum CookiePrivateError {
  /// Neither a `KeyRing` nor a `Key` is in the request extensions; responds 500.
  MissingKey,
}

impl Responder for CookiePrivateError {
  fn into_response(self) -> tako_rs_core::types::Response {
    match self {
      CookiePrivateError::MissingKey => (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Private cookie master key not found in request extensions",
      )
        .into_response(),
    }
  }
}

impl CookiePrivate {
  pub fn new(key: Key) -> Self {
    Self {
      jar: CookieJar::new(),
      key,
      ring: None,
    }
  }

  /// Encrypts with the ring's active key; reads also try its previous keys.
  pub fn with_ring(ring: crate::cookie_signed::KeyRing) -> Self {
    let key = ring.active().clone();
    Self {
      jar: CookieJar::new(),
      key,
      ring: Some(ring),
    }
  }

  /// Reads the cookies of every `Cookie` header.
  pub fn from_headers(headers: &HeaderMap, key: Key) -> Self {
    let mut jar = CookieJar::new();
    crate::cookie_jar::fill_jar_from_header(&mut jar, headers);

    Self {
      jar,
      key,
      ring: None,
    }
  }

  pub fn from_headers_with_ring(headers: &HeaderMap, ring: crate::cookie_signed::KeyRing) -> Self {
    let mut p = Self::from_headers(headers, ring.active().clone());
    p.ring = Some(ring);
    p
  }

  pub fn add(&mut self, cookie: Cookie<'static>) {
    self.jar.private_mut(&self.key).add(cookie);
  }

  pub fn remove(&mut self, name: &str) {
    self
      .jar
      .private_mut(&self.key)
      .remove(Cookie::from(name.to_owned()));
  }

  /// Decrypts `name` with the active key, then with each previous key of the ring.
  pub fn get(&self, name: &str) -> Option<Cookie<'static>> {
    if let Some(c) = self.jar.private(&self.key).get(name) {
      return Some(c);
    }
    if let Some(ring) = self.ring.as_ref() {
      for (_kid, key) in &ring.previous {
        if let Some(c) = self.jar.private(key).get(name) {
          return Some(c);
        }
      }
    }
    None
  }

  /// [`Self::get`] plus the id of the key that decrypted it, `default` without a ring.
  pub fn get_with_kid(&self, name: &str) -> Option<(Cookie<'static>, String)> {
    if let Some(c) = self.jar.private(&self.key).get(name) {
      let kid = self
        .ring
        .as_ref()
        .map_or_else(|| "default".to_string(), |r| r.active_kid().to_string());
      return Some((c, kid));
    }
    if let Some(ring) = self.ring.as_ref() {
      for (kid, key) in &ring.previous {
        if let Some(c) = self.jar.private(key).get(name) {
          return Some((c, kid.clone()));
        }
      }
    }
    None
  }

  pub fn get_value(&self, name: &str) -> Option<String> {
    self.get(name).map(|cookie| cookie.value().to_string())
  }

  /// Whether `name` exists and decrypts.
  pub fn contains(&self, name: &str) -> bool {
    self.get(name).is_some()
  }

  /// The underlying jar, which holds the encrypted values.
  pub fn inner(&self) -> &CookieJar {
    &self.jar
  }

  pub fn inner_mut(&mut self) -> &mut CookieJar {
    &mut self.jar
  }

  /// The active key.
  pub fn key(&self) -> &Key {
    &self.key
  }

  /// Appends encrypted additions and removals to a response as `Set-Cookie` headers.
  /// Invalid cookie names or attributes produce a 500 response; values are encrypted first.
  pub fn with_response(self, response: impl Responder) -> Response {
    crate::cookie_jar::append_delta(&self.jar, response.into_response())
  }

  fn extract_from_request(req: &Request) -> Result<Self, CookiePrivateError> {
    if let Some(ring) = req
      .extensions()
      .get::<crate::cookie_signed::KeyRing>()
      .cloned()
    {
      return Ok(Self::from_headers_with_ring(req.headers(), ring));
    }
    let key = req
      .extensions()
      .get::<Key>()
      .ok_or(CookiePrivateError::MissingKey)?
      .clone();

    Ok(Self::from_headers(req.headers(), key))
  }

  fn extract_from_parts(parts: &Parts) -> Result<Self, CookiePrivateError> {
    if let Some(ring) = parts
      .extensions
      .get::<crate::cookie_signed::KeyRing>()
      .cloned()
    {
      return Ok(Self::from_headers_with_ring(&parts.headers, ring));
    }
    let key = parts
      .extensions
      .get::<Key>()
      .ok_or(CookiePrivateError::MissingKey)?
      .clone();

    Ok(Self::from_headers(&parts.headers, key))
  }
}

impl Responder for CookiePrivate {
  fn into_response(self) -> Response {
    self.with_response(())
  }
}

impl<'a> FromRequest<'a> for CookiePrivate {
  type Error = CookiePrivateError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from_request(req))
  }
}

impl<'a> FromRequestParts<'a> for CookiePrivate {
  type Error = CookiePrivateError;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from_parts(parts))
  }
}
