use std::pin::Pin;

use anyhow::Result;
use http::request::Parts;

use crate::{
    extractors::{FromRequest, FromRequestParts},
    types::Request,
};

pub struct Path<'a>(pub &'a str);

impl<'a> FromRequest<'a> for Path<'a> {
    type Fut = Pin<Box<dyn Future<Output = Result<Self>> + Send + 'a>>;

    fn from_request(request: &'a mut Request) -> Self::Fut {
        Box::pin(async move { Ok(Path(request.uri().path())) })
    }
}

impl<'a> FromRequestParts<'a> for Path<'a> {
    type Fut = Pin<Box<dyn Future<Output = Result<Self>> + Send + 'a>>;

    fn from_request_parts(parts: &'a mut Parts) -> Self::Fut {
        Box::pin(async move { Ok(Path(parts.uri.path())) })
    }
}
