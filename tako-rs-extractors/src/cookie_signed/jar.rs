use cookie::Cookie;
use cookie::CookieJar;
use cookie::Key;
use http::HeaderMap;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Response;

use crate::cookie_signed::KeyRing;

/// A cookie jar that signs on write and verifies on read. Return it, or use
/// [`Self::with_response`], to send its changes as `Set-Cookie` headers.
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
  ring: Option<KeyRing>,
}

impl CookieSigned {
  pub fn new(key: Key) -> Self {
    Self {
      jar: CookieJar::new(),
      key,
      ring: None,
    }
  }

  /// Signs with the ring's active key; reads also try its previous keys.
  pub fn with_ring(ring: KeyRing) -> Self {
    let key = ring.active.clone();
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

  pub fn from_headers_with_ring(headers: &HeaderMap, ring: KeyRing) -> Self {
    let mut signed = Self::from_headers(headers, ring.active.clone());
    signed.ring = Some(ring);
    signed
  }

  /// [`Self::get`] plus the id of the key that verified it, `default` without a ring.
  pub fn get_with_kid(&self, name: &str) -> Option<(Cookie<'static>, String)> {
    if let Some(c) = self.jar.signed(&self.key).get(name) {
      let kid = self
        .ring
        .as_ref()
        .map_or_else(|| "default".to_string(), |r| r.active_kid.clone());
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

  pub fn add(&mut self, cookie: Cookie<'static>) {
    self.jar.signed_mut(&self.key).add(cookie);
  }

  pub fn remove(&mut self, name: &str) {
    self
      .jar
      .signed_mut(&self.key)
      .remove(Cookie::from(name.to_owned()));
  }

  /// Verifies `name` with the active key, then with each previous key of the ring.
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

  /// The underlying jar, which holds the signed values.
  pub fn inner(&self) -> &CookieJar {
    &self.jar
  }

  pub fn inner_mut(&mut self) -> &mut CookieJar {
    &mut self.jar
  }

  pub fn get_value(&self, name: &str) -> Option<String> {
    self.get(name).map(|cookie| cookie.value().to_string())
  }

  /// Whether `name` exists and verifies.
  pub fn contains(&self, name: &str) -> bool {
    self.get(name).is_some()
  }

  /// The active key.
  pub fn key(&self) -> &Key {
    &self.key
  }

  /// Appends the signed additions and removals to a response as `Set-Cookie` headers.
  /// Invalid cookie names, values, or attributes produce a 500 response.
  pub fn with_response(self, response: impl Responder) -> Response {
    crate::cookie_jar::append_delta(&self.jar, response.into_response())
  }
}

impl Responder for CookieSigned {
  fn into_response(self) -> Response {
    self.with_response(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn revoked_key_no_longer_verifies() {
    let active = Key::generate();
    let old = Key::generate();
    let mut ring = KeyRing::new("v2", active.clone()).with_previous("v1", old.clone());

    // Sign with the old key, then revoke it.
    let mut signing = CookieSigned::with_ring(ring.clone());
    signing.key = old.clone();
    signing.add(Cookie::new("hello", "world"));
    let cookie_str = signing.jar.iter().next().unwrap().to_string();

    let headers = {
      let mut h = HeaderMap::new();
      h.insert(http::header::COOKIE, cookie_str.parse().unwrap());
      h
    };

    let signed_before = CookieSigned::from_headers_with_ring(&headers, ring.clone());
    assert!(signed_before.get("hello").is_some());

    ring.revoke("v1");
    let signed_after = CookieSigned::from_headers_with_ring(&headers, ring);
    assert!(signed_after.get("hello").is_none());
  }
}
