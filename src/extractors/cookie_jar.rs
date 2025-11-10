//! Cookie extraction and management for HTTP requests.
//!
//! This module provides the [`CookieJar`] extractor that wraps the `cookie` crate's
//! `CookieJar` and integrates with the application's request lifecycle. It allows
//! extracting, adding, removing, and retrieving cookies from HTTP requests.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::cookie_jar::CookieJar;
//! use tako::types::Request;
//! use cookie::Cookie;
//!
//! async fn handle_cookies(jar: CookieJar) {
//!     // Get a cookie value
//!     if let Some(session_cookie) = jar.get("session_id") {
//!         println!("Session ID: {}", session_cookie.value());
//!     }
//!
//!     // Iterate over all cookies
//!     for cookie in jar.iter() {
//!         println!("Cookie: {}={}", cookie.name(), cookie.value());
//!     }
//! }
//! ```
use cookie::{Cookie, CookieJar as RawJar};
use http::{HeaderMap, header::COOKIE, request::Parts};
use std::{convert::Infallible, future::ready};

use crate::{
  extractors::{FromRequest, FromRequestParts},
  types::Request,
};

/// A wrapper around the `cookie::CookieJar` that provides methods for managing cookies
/// in HTTP requests and responses.
///
/// This struct allows adding, removing, and retrieving cookies, as well as creating
/// a `CookieJar` instance from HTTP headers. It integrates with the framework's
/// extractor system to automatically parse cookies from incoming requests.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::cookie_jar::CookieJar;
/// use cookie::Cookie;
///
/// let mut jar = CookieJar::new();
/// jar.add(Cookie::new("name", "value"));
///
/// if let Some(cookie) = jar.get("name") {
///     assert_eq!(cookie.value(), "value");
/// }
/// ```
pub struct CookieJar(RawJar);

impl CookieJar {
  /// Creates a new, empty `CookieJar` instance.
  pub fn new() -> Self {
    Self(RawJar::new())
  }

  /// Initializes a `CookieJar` instance from the `Cookie` header in the provided HTTP headers.
  pub fn from_headers(headers: &HeaderMap) -> Self {
    let mut jar = RawJar::new();

    if let Some(val) = headers.get(COOKIE).and_then(|v| v.to_str().ok()) {
      for s in val.split(';') {
        if let Ok(c) = Cookie::parse(s.trim()) {
          jar.add_original(c.into_owned());
        }
      }
    }

    Self(jar)
  }

  /// Inserts a cookie into the `CookieJar`.
  pub fn add(&mut self, cookie: Cookie<'static>) {
    self.0.add(cookie);
  }

  /// Deletes a cookie from the `CookieJar` by its name.
  pub fn remove(&mut self, name: &str) {
    self.0.remove(Cookie::from(name.to_owned()));
  }

  /// Fetches a cookie from the `CookieJar` by its name.
  pub fn get(&self, name: &str) -> Option<&Cookie<'_>> {
    self.0.get(name)
  }

  /// Provides an iterator over all cookies in the `CookieJar`.
  pub fn iter(&self) -> impl Iterator<Item = &Cookie<'static>> {
    self.0.iter()
  }
}

impl<'a> FromRequest<'a> for CookieJar {
  type Error = Infallible;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    ready(Ok(CookieJar::from_headers(req.headers())))
  }
}

impl<'a> FromRequestParts<'a> for CookieJar {
  type Error = Infallible;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    ready(Ok(CookieJar::from_headers(&parts.headers)))
  }
}
