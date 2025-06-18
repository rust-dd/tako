use std::pin::Pin;

use anyhow::Result;
use http::request::Parts;

use crate::{
    extractors::{FromRequest, FromRequestParts},
    types::Request,
};

pub struct HeaderMap<'a>(pub &'a hyper::HeaderMap);

impl<'a> FromRequest<'a> for HeaderMap<'a> {
    type Fut = Pin<Box<dyn Future<Output = Result<Self>> + Send + 'a>>;

    fn from_request(req: &'a mut Request) -> Self::Fut {
        Box::pin(async move { Ok(HeaderMap(req.headers())) })
    }
}

impl<'a> FromRequestParts<'a> for HeaderMap<'a> {
    type Fut = Pin<Box<dyn Future<Output = Result<Self>> + Send + 'a>>;

    fn from_request_parts(parts: &'a mut Parts) -> Self::Fut {
        Box::pin(async move { Ok(HeaderMap(&parts.headers)) })
    }
}
