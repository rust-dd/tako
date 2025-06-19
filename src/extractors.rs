use anyhow::Result;
use http::request::Parts;

use crate::types::Request;

pub mod bytes;
pub mod header_map;
pub mod json;
pub mod params;
pub mod path;
pub mod query;

pub trait FromRequest<'a>: Sized {
    type Fut: Future<Output = Result<Self>> + Send + 'a;

    fn from_request(req: &'a mut Request) -> Self::Fut;
}

pub trait FromRequestParts<'a>: Sized {
    type Fut: Future<Output = Result<Self>> + Send + 'a;

    fn from_request_parts(parts: &'a mut Parts) -> Self::Fut;
}
