use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use http::request::Parts;

use crate::{
    extractors::{FromRequest, FromRequestParts},
    types::Request,
};

pub struct Basic {
    pub username: String,
    pub password: String,
    pub raw: String,
}

impl<'a> FromRequest<'a> for Basic {
    fn from_request(req: &'a Request) -> Result<Self> {
        let token = req
            .headers()
            .get("Authorization")
            .and_then(|v| v.to_str().ok());

        match token {
            Some(token) if token.starts_with("Basic") => {
                let encoded = &token[6..];
                let decoded = STANDARD.decode(encoded)?;
                let decoded = str::from_utf8(&decoded)?;

                let parts = decoded.splitn(2, ":").collect::<Vec<_>>();
                if parts.len() != 2 {
                    Err(anyhow::anyhow!("Invalid Basic auth token"))
                } else {
                    Ok(Basic {
                        username: parts[0].to_string(),
                        password: parts[1].to_string(),
                        raw: token.to_string(),
                    })
                }
            }
            _ => Err(anyhow::anyhow!("Missing Basic auth token")),
        }
    }
}

impl<'a> FromRequestParts<'a> for Basic {
    fn from_request_parts(parts: &'a mut Parts) -> Result<Self> {
        let token = parts
            .headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok());

        match token {
            Some(token) if token.starts_with("Basic") => {
                let encoded = &token[6..];
                let decoded = STANDARD.decode(encoded)?;
                let decoded = str::from_utf8(&decoded)?;

                let parts = decoded.splitn(2, ":").collect::<Vec<_>>();
                if parts.len() != 2 {
                    Err(anyhow::anyhow!("Invalid Basic auth token"))
                } else {
                    Ok(Basic {
                        username: parts[0].to_string(),
                        password: parts[1].to_string(),
                        raw: token.to_string(),
                    })
                }
            }
            _ => Err(anyhow::anyhow!("Missing Basic auth token")),
        }
    }
}
