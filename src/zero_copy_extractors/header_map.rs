use std::convert::Infallible;

use futures_util::future::ready;

use crate::extractors::FromRequest;
use crate::extractors::FromRequestParts;

pub struct HeaderMapBorrowed<'a>(pub &'a http::HeaderMap);

impl<'a> FromRequest<'a> for HeaderMapBorrowed<'a> {
  type Error = Infallible;

  fn from_request(
    req: &'a mut crate::types::Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    ready(Ok(HeaderMapBorrowed(req.headers())))
  }
}

impl<'a> FromRequestParts<'a> for HeaderMapBorrowed<'a> {
  type Error = Infallible;

  fn from_request_parts(
    parts: &'a mut http::request::Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    ready(Ok(HeaderMapBorrowed(&parts.headers)))
  }
}
