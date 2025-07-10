use anyhow::Result;
use http::request::Parts;

use crate::{
    extractors::{FromRequest, FromRequestParts},
    types::Request,
};

/// Represents the Bearer authentication token extracted from a request.
pub struct Bearer {
    /// The token extracted from the Bearer auth header, without the "Bearer " prefix.
    pub token: String,
    /// The full Bearer token as received in the request, including the "Bearer " prefix.
    pub with_bearer: String,
}

impl<'a> FromRequest<'a> for Bearer {
    /// Extracts the Bearer authentication token from a full HTTP request.
    ///
    /// # Arguments
    /// * `req` - A reference to the HTTP request from which the Bearer token is extracted.
    ///
    /// # Returns
    /// * `Ok(Bearer)` - If a valid Bearer token is found in the "Authorization" header.
    /// * `Err` - If the "Authorization" header is missing or does not contain a valid Bearer token.
    fn from_request(req: &'a Request) -> Result<Self> {
        let token = req
            .headers()
            .get("Authorization")
            .and_then(|value| value.to_str().ok());

        match token {
            Some(token) if token.starts_with("Bearer ") => Ok(Bearer {
                token: token[7..].to_string(),
                with_bearer: token.to_string(),
            }),
            _ => Err(anyhow::anyhow!("Invalid bearer token")),
        }
    }
}

impl<'a> FromRequestParts<'a> for Bearer {
    /// Extracts the Bearer authentication token from the parts of an HTTP request.
    ///
    /// # Arguments
    /// * `parts` - A mutable reference to the HTTP request parts from which the Bearer token is extracted.
    ///
    /// # Returns
    /// * `Ok(Bearer)` - If a valid Bearer token is found in the "Authorization" header.
    /// * `Err` - If the "Authorization" header is missing or does not contain a valid Bearer token.
    fn from_request_parts(parts: &'a mut Parts) -> Result<Self> {
        let token = parts
            .headers
            .get("Authorization")
            .and_then(|value| value.to_str().ok());

        match token {
            Some(token) if token.starts_with("Bearer ") => Ok(Bearer {
                token: token[7..].to_string(),
                with_bearer: token.to_string(),
            }),
            _ => Err(anyhow::anyhow!("Invalid bearer token")),
        }
    }
}
