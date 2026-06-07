use cookie::Cookie;
use cookie::CookieJar;
use cookie::Key;
use http::HeaderMap;

use crate::cookie_signed::KeyRing;

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
    crate::cookie_jar::fill_jar_from_header(&mut jar, headers);

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

    // Before revocation: lookup succeeds via the previous key.
    let signed_before = CookieSigned::from_headers_with_ring(&headers, ring.clone());
    assert!(signed_before.get("hello").is_some());

    // After revocation: lookup fails.
    ring.revoke("v1");
    let signed_after = CookieSigned::from_headers_with_ring(&headers, ring);
    assert!(signed_after.get("hello").is_none());
  }
}
