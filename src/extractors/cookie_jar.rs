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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_jar::CookieJar;
    ///
    /// let jar = CookieJar::new();
    /// // jar is now ready to store cookies
    /// ```
    pub fn new() -> Self {
        Self(RawJar::new())
    }

    /// Creates a `CookieJar` instance from the `Cookie` header in the provided HTTP headers.
    ///
    /// Parses the `Cookie` header value and populates the jar with all valid cookies found.
    /// Invalid cookie strings are silently ignored.
    ///
    /// # Arguments
    ///
    /// * `headers` - HTTP headers containing the `Cookie` header to parse
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_jar::CookieJar;
    /// use http::HeaderMap;
    ///
    /// let mut headers = HeaderMap::new();
    /// headers.insert(
    ///     http::header::COOKIE,
    ///     "session_id=abc123; user_pref=dark_mode".parse().unwrap()
    /// );
    ///
    /// let jar = CookieJar::from_headers(&headers);
    /// assert!(jar.get("session_id").is_some());
    /// assert!(jar.get("user_pref").is_some());
    /// ```
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

    /// Adds a cookie to the `CookieJar`.
    ///
    /// The cookie will be stored in the jar and can be retrieved later using [`get`].
    ///
    /// [`get`]: CookieJar::get
    ///
    /// # Arguments
    ///
    /// * `cookie` - The cookie to add to the jar
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_jar::CookieJar;
    /// use cookie::Cookie;
    ///
    /// let mut jar = CookieJar::new();
    /// jar.add(Cookie::new("theme", "dark"));
    ///
    /// assert!(jar.get("theme").is_some());
    /// ```
    pub fn add(&mut self, cookie: Cookie<'static>) {
        self.0.add(cookie);
    }

    /// Removes a cookie from the `CookieJar` by its name.
    ///
    /// If a cookie with the specified name exists, it will be removed from the jar.
    /// If no such cookie exists, this method has no effect.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the cookie to remove
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_jar::CookieJar;
    /// use cookie::Cookie;
    ///
    /// let mut jar = CookieJar::new();
    /// jar.add(Cookie::new("temp", "data"));
    /// jar.remove("temp");
    ///
    /// assert!(jar.get("temp").is_none());
    /// ```
    pub fn remove(&mut self, name: &str) {
        self.0.remove(Cookie::from(name.to_owned()));
    }

    /// Retrieves a cookie from the `CookieJar` by its name.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the cookie to retrieve
    ///
    /// # Returns
    ///
    /// An `Option` containing a reference to the cookie if it exists, or `None` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_jar::CookieJar;
    /// use cookie::Cookie;
    ///
    /// let mut jar = CookieJar::new();
    /// jar.add(Cookie::new("user_id", "12345"));
    ///
    /// if let Some(cookie) = jar.get("user_id") {
    ///     assert_eq!(cookie.value(), "12345");
    /// }
    ///
    /// assert!(jar.get("nonexistent").is_none());
    /// ```
    pub fn get(&self, name: &str) -> Option<&Cookie<'_>> {
        self.0.get(name)
    }

    /// Returns an iterator over all cookies in the `CookieJar`.
    ///
    /// The iterator yields references to all cookies currently stored in the jar.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_jar::CookieJar;
    /// use cookie::Cookie;
    ///
    /// let mut jar = CookieJar::new();
    /// jar.add(Cookie::new("cookie1", "value1"));
    /// jar.add(Cookie::new("cookie2", "value2"));
    ///
    /// let count = jar.iter().count();
    /// assert_eq!(count, 2);
    ///
    /// for cookie in jar.iter() {
    ///     println!("{}={}", cookie.name(), cookie.value());
    /// }
    /// ```
    pub fn iter(&self) -> impl Iterator<Item = &Cookie<'static>> {
        self.0.iter()
    }
}

impl<'a> FromRequest<'a> for CookieJar {
    type Error = Infallible;

    /// Extracts a `CookieJar` from an HTTP request.
    ///
    /// This implementation reads the `Cookie` header from the request and parses
    /// all valid cookies into a `CookieJar` instance.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::{FromRequest, cookie_jar::CookieJar};
    /// use tako::types::Request;
    ///
    /// async fn handler(mut req: Request) -> Result<(), Box<dyn std::error::Error>> {
    ///     let jar = CookieJar::from_request(&mut req).await?;
    ///
    ///     if let Some(session) = jar.get("session_id") {
    ///         println!("Session: {}", session.value());
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Ok(CookieJar::from_headers(req.headers())))
    }
}

impl<'a> FromRequestParts<'a> for CookieJar {
    type Error = Infallible;

    /// Extracts a `CookieJar` from HTTP request parts.
    ///
    /// This implementation reads the `Cookie` header from the request parts and parses
    /// all valid cookies into a `CookieJar` instance.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::{FromRequestParts, cookie_jar::CookieJar};
    /// use http::request::Parts;
    ///
    /// async fn handler(mut parts: Parts) -> Result<(), Box<dyn std::error::Error>> {
    ///     let jar = CookieJar::from_request_parts(&mut parts).await?;
    ///
    ///     // Use the cookie jar
    ///     let cookie_count = jar.iter().count();
    ///     println!("Found {} cookies", cookie_count);
    ///
    ///     Ok(())
    /// }
    /// ```
    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Ok(CookieJar::from_headers(&parts.headers)))
    }
}
