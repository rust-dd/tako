use cookie::{Cookie, CookieJar, Key};
use http::{HeaderMap, StatusCode, header::COOKIE, request::Parts};
use std::future::ready;

use crate::{
    extractors::{FromRequest, FromRequestParts},
    responder::Responder,
    types::Request,
};

/// A wrapper that provides methods for managing HMAC-signed cookies in HTTP requests and responses.
///
/// Signed cookies use HMAC (Hash-based Message Authentication Code) to ensure
/// that cookie values haven't been tampered with, while keeping the content readable.
pub struct CookieSigned {
    jar: CookieJar,
    key: Key,
}

/// Error type for signed cookie extraction.
#[derive(Debug)]
pub enum CookieSignedError {
    MissingKey,
    InvalidKey,
    VerificationFailed(String),
    InvalidCookieFormat,
    InvalidSignature(String),
}

impl Responder for CookieSignedError {
    fn into_response(self) -> crate::types::Response {
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
                format!("Failed to verify signed cookie: {}", err),
            )
                .into_response(),
            CookieSignedError::InvalidCookieFormat => {
                (StatusCode::BAD_REQUEST, "Invalid cookie format in request").into_response()
            }
            CookieSignedError::InvalidSignature(cookie_name) => (
                StatusCode::BAD_REQUEST,
                format!("Invalid signature for cookie: {}", cookie_name),
            )
                .into_response(),
        }
    }
}

impl CookieSigned {
    /// Creates a new `CookieSigned` instance with the given master key.
    ///
    /// # Parameters
    /// - `key`: The master key used for signing and verifying cookies.
    ///
    /// # Returns
    /// A new `CookieSigned` instance.
    pub fn new(key: Key) -> Self {
        Self {
            jar: CookieJar::new(),
            key,
        }
    }

    /// Creates a `CookieSigned` instance from HTTP headers and a master key.
    ///
    /// # Parameters
    /// - `headers`: A reference to the HTTP headers containing the `Cookie` header.
    /// - `key`: The master key used for verifying cookie signatures.
    ///
    /// # Returns
    /// A `CookieSigned` populated with cookies from the `Cookie` header.
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

    /// Adds a signed cookie to the jar.
    ///
    /// The cookie will be signed with HMAC when serialized.
    ///
    /// # Parameters
    /// - `cookie`: The cookie to add and sign.
    pub fn add(&mut self, cookie: Cookie<'static>) {
        self.jar.signed_mut(&self.key).add(cookie);
    }

    /// Removes a signed cookie from the jar by its name.
    ///
    /// # Parameters
    /// - `name`: The name of the cookie to remove.
    pub fn remove(&mut self, name: &str) {
        self.jar
            .signed_mut(&self.key)
            .remove(Cookie::from(name.to_owned()));
    }

    /// Retrieves and verifies a signed cookie from the jar by its name.
    ///
    /// # Parameters
    /// - `name`: The name of the cookie to retrieve.
    ///
    /// # Returns
    /// An `Option` containing the verified cookie if it exists and
    /// has a valid signature, or `None` otherwise.
    pub fn get(&self, name: &str) -> Option<Cookie<'static>> {
        self.jar.signed(&self.key).get(name)
    }

    /// Gets the inner `CookieJar` for advanced operations.
    ///
    /// # Returns
    /// A reference to the inner `CookieJar`.
    pub fn inner(&self) -> &CookieJar {
        &self.jar
    }

    /// Gets a mutable reference to the inner `CookieJar` for advanced operations.
    ///
    /// # Returns
    /// A mutable reference to the inner `CookieJar`.
    pub fn inner_mut(&mut self) -> &mut CookieJar {
        &mut self.jar
    }

    /// Verifies if a cookie with the given name exists and has a valid signature.
    ///
    /// # Parameters
    /// - `name`: The name of the cookie to verify.
    ///
    /// # Returns
    /// `true` if the cookie exists and has a valid signature, `false` otherwise.
    pub fn verify(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Gets the value of a signed cookie after verification.
    ///
    /// # Parameters
    /// - `name`: The name of the cookie whose value to retrieve.
    ///
    /// # Returns
    /// An `Option` containing the cookie value if the cookie exists and has a valid signature.
    pub fn get_value(&self, name: &str) -> Option<String> {
        self.get(name).map(|cookie| cookie.value().to_string())
    }

    /// Checks if a signed cookie with the given name exists and has a valid signature.
    ///
    /// # Parameters
    /// - `name`: The name of the cookie to check.
    ///
    /// # Returns
    /// `true` if the cookie exists and has a valid signature, `false` otherwise.
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Gets the key used for signed cookie operations.
    ///
    /// # Returns
    /// A reference to the signing key.
    pub fn key(&self) -> &Key {
        &self.key
    }

    /// Extracts signed cookies from a request using a master key from extensions.
    fn extract_from_request(req: &Request) -> Result<Self, CookieSignedError> {
        let key = req
            .extensions()
            .get::<Key>()
            .ok_or(CookieSignedError::MissingKey)?
            .clone();

        Ok(Self::from_headers(req.headers(), key))
    }

    /// Extracts signed cookies from request parts using a master key from extensions.
    fn extract_from_parts(parts: &Parts) -> Result<Self, CookieSignedError> {
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
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_request(req))
    }
}

impl<'a> FromRequestParts<'a> for CookieSigned {
    type Error = CookieSignedError;

    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_parts(parts))
    }
}
