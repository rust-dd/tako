use anyhow::Result;
use http::request::Parts;

use crate::{
    extractors::{FromRequest, FromRequestParts},
    types::Request,
};

pub struct Bearer {
    pub token: String,
    pub with_bearer: String,
}

impl<'a> FromRequest<'a> for Bearer {
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
