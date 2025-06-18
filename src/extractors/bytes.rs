use std::pin::Pin;

use anyhow::Result;
use hyper::body::Incoming;

use crate::{extractors::FromRequest, types::Request};

pub struct Bytes<'a>(pub &'a Incoming);

impl<'a> FromRequest<'a> for Bytes<'a> {
    type Fut = Pin<Box<dyn Future<Output = Result<Self>> + Send + 'a>>;

    fn from_request(req: &'a mut Request) -> Self::Fut {
        Box::pin(async move { Ok(Bytes(req.body())) })
    }
}
