use base64::{Engine, engine::general_purpose::STANDARD};
use http::{StatusCode, request::Parts};
use std::future::ready;

use crate::{
    extractors::{FromRequest, FromRequestParts},
    responder::Responder,
    types::Request,
};

/// Represents the Basic authentication credentials extracted from a request.
pub struct Basic {
    /// The username extracted from the Basic auth token.
    pub username: String,
    /// The password extracted from the Basic auth token.
    pub password: String,
    /// The raw Basic auth token as received in the request.
    pub raw: String,
}

/// Error type for Basic authentication extraction.
#[derive(Debug)]
pub enum BasicAuthError {
    MissingAuthHeader,
    InvalidAuthHeader,
    InvalidBasicFormat,
    InvalidBase64,
    InvalidUtf8,
    InvalidCredentialsFormat,
}

impl Responder for BasicAuthError {
    fn into_response(self) -> crate::types::Response {
        let (status, message) = match self {
            BasicAuthError::MissingAuthHeader => {
                (StatusCode::UNAUTHORIZED, "Missing Authorization header")
            }
            BasicAuthError::InvalidAuthHeader => {
                (StatusCode::UNAUTHORIZED, "Invalid Authorization header")
            }
            BasicAuthError::InvalidBasicFormat => (
                StatusCode::UNAUTHORIZED,
                "Authorization header is not Basic auth",
            ),
            BasicAuthError::InvalidBase64 => (
                StatusCode::UNAUTHORIZED,
                "Invalid Base64 encoding in Basic auth",
            ),
            BasicAuthError::InvalidUtf8 => (
                StatusCode::UNAUTHORIZED,
                "Invalid UTF-8 in Basic auth credentials",
            ),
            BasicAuthError::InvalidCredentialsFormat => (
                StatusCode::UNAUTHORIZED,
                "Invalid credentials format in Basic auth",
            ),
        };
        (status, message).into_response()
    }
}

impl Basic {
    fn extract_from_headers(headers: &http::HeaderMap) -> Result<Self, BasicAuthError> {
        let auth_header = headers
            .get("Authorization")
            .ok_or(BasicAuthError::MissingAuthHeader)?;

        let auth_str = auth_header
            .to_str()
            .map_err(|_| BasicAuthError::InvalidAuthHeader)?;

        if !auth_str.starts_with("Basic ") {
            return Err(BasicAuthError::InvalidBasicFormat);
        }

        let encoded = &auth_str[6..];
        let decoded = STANDARD
            .decode(encoded)
            .map_err(|_| BasicAuthError::InvalidBase64)?;

        let decoded_str = std::str::from_utf8(&decoded).map_err(|_| BasicAuthError::InvalidUtf8)?;

        let parts: Vec<&str> = decoded_str.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(BasicAuthError::InvalidCredentialsFormat);
        }

        Ok(Basic {
            username: parts[0].to_string(),
            password: parts[1].to_string(),
            raw: auth_str.to_string(),
        })
    }
}

impl<'a> FromRequest<'a> for Basic {
    type Error = BasicAuthError;

    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_headers(req.headers()))
    }
}

impl<'a> FromRequestParts<'a> for Basic {
    type Error = BasicAuthError;

    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_headers(&parts.headers))
    }
}
