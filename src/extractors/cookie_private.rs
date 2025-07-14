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
/// values cannot be read or tampered with by clients.
pub struct CookiePrivate {
    jar: CookieJar,
    key: Key,
}

/// Error type for private cookie extraction.
#[derive(Debug)]
pub enum CookiePrivateError {
    MissingKey,
    InvalidKey,
    DecryptionFailed(String),
    InvalidCookieFormat,
}

impl Responder for CookiePrivateError {
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
    ///
    /// # Parameters
    /// - `key`: The master key used for encrypting and decrypting cookies.
    ///
    /// # Returns
    /// A new `CookiePrivate` instance.
    pub fn new(key: Key) -> Self {
        Self {
            jar: CookieJar::new(),
            key,
        }
    }

    /// Creates a `CookiePrivate` instance from HTTP headers and a master key.
    ///
    /// # Parameters
    /// - `headers`: A reference to the HTTP headers containing the `Cookie` header.
    /// - `key`: The master key used for decrypting cookies.
    ///
    /// # Returns
    /// A `CookiePrivate` populated with cookies from the `Cookie` header.
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
    ///
    /// The cookie will be encrypted when serialized.
    ///
    /// # Parameters
    /// - `cookie`: The cookie to add and encrypt.
    pub fn add(&mut self, cookie: Cookie<'static>) {
        self.jar.private_mut(&self.key).add(cookie);
    }

    /// Removes a private cookie from the jar by its name.
    ///
    /// # Parameters
    /// - `name`: The name of the cookie to remove.
    pub fn remove(&mut self, name: &str) {
        self.jar
            .private_mut(&self.key)
            .remove(Cookie::from(name.to_owned()));
    }

    /// Retrieves and decrypts a private cookie from the jar by its name.
    ///
    /// # Parameters
    /// - `name`: The name of the cookie to retrieve.
    ///
    /// # Returns
    /// An `Option` containing the decrypted cookie if it exists and
    /// can be successfully decrypted, or `None` otherwise.
    pub fn get(&self, name: &str) -> Option<Cookie<'static>> {
        self.jar.private(&self.key).get(name)
    }

    /// Gets the value of a private cookie after decryption.
    ///
    /// # Parameters
    /// - `name`: The name of the cookie whose value to retrieve.
    ///
    /// # Returns
    /// An `Option` containing the cookie value if the cookie exists and can be decrypted.
    pub fn get_value(&self, name: &str) -> Option<String> {
        self.get(name).map(|cookie| cookie.value().to_string())
    }

    /// Checks if a private cookie with the given name exists and can be decrypted.
    ///
    /// # Parameters
    /// - `name`: The name of the cookie to check.
    ///
    /// # Returns
    /// `true` if the cookie exists and can be decrypted, `false` otherwise.
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
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

    /// Gets the key used for private cookie operations.
    ///
    /// # Returns
    /// A reference to the encryption key.
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
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_request(req))
    }
}

impl<'a> FromRequestParts<'a> for CookiePrivate {
    type Error = CookiePrivateError;

    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_parts(parts))
    }
}
