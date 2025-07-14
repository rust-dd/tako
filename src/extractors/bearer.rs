use http::{StatusCode, request::Parts};
use std::future::ready;

use crate::{
    extractors::{FromRequest, FromRequestParts},
    responder::Responder,
    types::Request,
};

/// Represents the Bearer authentication token extracted from a request.
pub struct Bearer {
    /// The token extracted from the Bearer auth header, without the "Bearer " prefix.
    pub token: String,
    /// The full Bearer token as received in the request, including the "Bearer " prefix.
    pub with_bearer: String,
}

/// Error type for Bearer authentication extraction.
#[derive(Debug)]
pub enum BearerAuthError {
    MissingAuthHeader,
    InvalidAuthHeader,
    InvalidBearerFormat,
    EmptyToken,
}

impl Responder for BearerAuthError {
    fn into_response(self) -> crate::types::Response {
        let (status, message) = match self {
            BearerAuthError::MissingAuthHeader => {
                (StatusCode::UNAUTHORIZED, "Missing Authorization header")
            }
            BearerAuthError::InvalidAuthHeader => {
                (StatusCode::UNAUTHORIZED, "Invalid Authorization header")
            }
            BearerAuthError::InvalidBearerFormat => (
                StatusCode::UNAUTHORIZED,
                "Authorization header is not Bearer token",
            ),
            BearerAuthError::EmptyToken => (StatusCode::UNAUTHORIZED, "Bearer token is empty"),
        };
        (status, message).into_response()
    }
}

impl Bearer {
    fn extract_from_headers(headers: &http::HeaderMap) -> Result<Self, BearerAuthError> {
        let auth_header = headers
            .get("Authorization")
            .ok_or(BearerAuthError::MissingAuthHeader)?;

        let auth_str = auth_header
            .to_str()
            .map_err(|_| BearerAuthError::InvalidAuthHeader)?;

        if !auth_str.starts_with("Bearer ") {
            return Err(BearerAuthError::InvalidBearerFormat);
        }

        let token = &auth_str[7..];
        if token.is_empty() {
            return Err(BearerAuthError::EmptyToken);
        }

        Ok(Bearer {
            token: token.to_string(),
            with_bearer: auth_str.to_string(),
        })
    }
}

impl<'a> FromRequest<'a> for Bearer {
    type Error = BearerAuthError;

    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_headers(req.headers()))
    }
}

impl<'a> FromRequestParts<'a> for Bearer {
    type Error = BearerAuthError;

    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_headers(&parts.headers))
    }
}
