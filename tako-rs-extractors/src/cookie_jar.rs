//! The request's cookies, parsed from every `Cookie` header; extraction never fails.
//! Return the jar, or `jar.with_response(body)`, to send additions and removals as `Set-Cookie`.
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

use std::convert::Infallible;

use cookie::Cookie;
use cookie::CookieJar as RawJar;
use http::HeaderMap;
use http::HeaderValue;
use http::StatusCode;
use http::header::COOKIE;
use http::header::SET_COOKIE;
use http::request::Parts;
use tako_rs_core::extractors::Entries;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

/// Adds well-formed cookies from every `Cookie` header to `jar` as originals.
pub(crate) fn fill_jar_from_header(jar: &mut RawJar, headers: &HeaderMap) {
  for val in headers
    .get_all(COOKIE)
    .iter()
    .filter_map(|v| v.to_str().ok())
  {
    for s in val.split(';') {
      if let Ok(c) = Cookie::parse(s.trim()) {
        jar.add_original(c.into_owned());
      }
    }
  }
}

pub(crate) fn append_delta(jar: &RawJar, mut response: Response) -> Response {
  for cookie in jar.delta() {
    if !valid_response_cookie(cookie) {
      return (StatusCode::INTERNAL_SERVER_ERROR, "Invalid response cookie").into_response();
    }
    let Ok(value) = HeaderValue::from_str(&cookie.to_string()) else {
      return (StatusCode::INTERNAL_SERVER_ERROR, "Invalid response cookie").into_response();
    };
    response.headers_mut().append(SET_COOKIE, value);
  }
  response
}

fn valid_response_cookie(cookie: &Cookie<'_>) -> bool {
  let name_valid = !cookie.name().is_empty()
    && cookie
      .name()
      .bytes()
      .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte));
  let value = cookie.value();
  let value = value
    .strip_prefix('"')
    .and_then(|value| value.strip_suffix('"'))
    .unwrap_or(value);
  // RFC 6265 cookie-octet excludes separators that could become cookie attributes.
  let value_valid = value
    .bytes()
    .all(|byte| matches!(byte, 0x21 | 0x23..=0x2b | 0x2d..=0x3a | 0x3c..=0x5b | 0x5d..=0x7e));
  let attributes_valid = [cookie.path(), cookie.domain()]
    .into_iter()
    .flatten()
    .all(|value| {
      value
        .bytes()
        .all(|byte| matches!(byte, 0x20..=0x3a | 0x3c..=0x7e))
    });
  name_valid && value_valid && attributes_valid
}

/// A `cookie::CookieJar` holding the request's cookies. Return it from the handler to emit
/// its changes, or use [`Self::with_response`] to include a response body.
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

impl Default for CookieJar {
  fn default() -> Self {
    Self::new()
  }
}

impl CookieJar {
  pub fn new() -> Self {
    Self(RawJar::new())
  }

  /// Parses every `Cookie` header, skipping malformed cookies.
  pub fn from_headers(headers: &HeaderMap) -> Self {
    let mut jar = RawJar::new();
    fill_jar_from_header(&mut jar, headers);
    Self(jar)
  }

  pub fn add(&mut self, cookie: Cookie<'static>) {
    self.0.add(cookie);
  }

  /// Removes `name`, keeping the `Path` and `Domain` of a cookie added with them so the
  /// removal targets the same cookie.
  pub fn remove(&mut self, name: &str) {
    let owned_name = name.to_owned();
    let mut marker = Cookie::from(owned_name);
    if let Some(existing) = self.0.get(name) {
      if let Some(path) = existing.path() {
        marker.set_path(path.to_owned());
      }
      if let Some(domain) = existing.domain() {
        marker.set_domain(domain.to_owned());
      }
    }
    self.0.remove(marker);
  }

  pub fn get(&self, name: &str) -> Option<&Cookie<'_>> {
    self.0.get(name)
  }

  pub fn iter(&self) -> impl Iterator<Item = &Cookie<'static>> {
    self.0.iter()
  }

  /// Appends additions and removals to `response` as separate `Set-Cookie` headers.
  /// Unchanged request cookies are omitted. Invalid names, values, or attributes produce a 500.
  pub fn with_response(self, response: impl Responder) -> Response {
    append_delta(&self.0, response.into_response())
  }
}

impl Responder for CookieJar {
  fn into_response(self) -> Response {
    self.with_response(())
  }
}

impl<'a> FromRequest<'a> for CookieJar {
  type Error = Infallible;
  const ENTRIES: Entries = Entries::NONE;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Ok(CookieJar::from_headers(req.headers())))
  }
}

impl<'a> FromRequestParts<'a> for CookieJar {
  type Error = Infallible;
  const ENTRIES: Entries = Entries::NONE;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Ok(CookieJar::from_headers(&parts.headers)))
  }
}
