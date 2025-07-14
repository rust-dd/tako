/// This module provides functionality for extracting and managing cookies in HTTP requests.
/// It includes a `CookieJar` struct that wraps the `cookie` crate's `CookieJar`
/// and integrates with the application's request lifecycle.
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
/// a `CookieJar` instance from HTTP headers.
pub struct CookieJar(RawJar);

impl CookieJar {
    /// Creates a new, empty `CookieJar` instance.
    ///
    /// # Returns
    /// A new `CookieJar` with no cookies.
    pub fn new() -> Self {
        Self(RawJar::new())
    }

    /// Creates a `CookieJar` instance from the `Cookie` header in the provided HTTP headers.
    ///
    /// # Parameters
    /// - `headers`: A reference to the HTTP headers containing the `Cookie` header.
    ///
    /// # Returns
    /// A `CookieJar` populated with cookies parsed from the `Cookie` header.
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
    /// # Parameters
    /// - `cookie`: The cookie to add.
    pub fn add(&mut self, cookie: Cookie<'static>) {
        self.0.add(cookie);
    }

    /// Removes a cookie from the `CookieJar` by its name.
    ///
    /// # Parameters
    /// - `name`: The name of the cookie to remove.
    pub fn remove(&mut self, name: &str) {
        self.0.remove(Cookie::from(name.to_owned()));
    }

    /// Retrieves a cookie from the `CookieJar` by its name.
    ///
    /// # Parameters
    /// - `name`: The name of the cookie to retrieve.
    ///
    /// # Returns
    /// An `Option` containing a reference to the cookie if it exists, or `None` otherwise.
    pub fn get(&self, name: &str) -> Option<&Cookie<'_>> {
        self.0.get(name)
    }

    /// Returns an iterator over all cookies in the `CookieJar`.
    ///
    /// # Returns
    /// An iterator yielding references to the cookies.
    pub fn iter(&self) -> impl Iterator<Item = &Cookie<'static>> {
        self.0.iter()
    }
}

impl<'a> FromRequest<'a> for CookieJar {
    type Error = Infallible;

    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Ok(CookieJar::from_headers(req.headers())))
    }
}

impl<'a> FromRequestParts<'a> for CookieJar {
    type Error = Infallible;

    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Ok(CookieJar::from_headers(&parts.headers)))
    }
}
