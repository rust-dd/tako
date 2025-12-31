use std::convert::Infallible;
use std::future::ready;

use crate::extractors::FromRequest;
use crate::extractors::FromRequestParts;

/// Zero-copy path extractor borrowing the request URI path.
pub struct PathBorrowed<'a>(pub &'a str);

impl<'a> FromRequest<'a> for PathBorrowed<'a> {
  type Error = Infallible;

  fn from_request(
    req: &'a mut crate::types::Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    // The returned &str borrows from the request for the same lifetime 'a.
    ready(Ok(PathBorrowed(req.uri().path())))
  }
}

impl<'a> FromRequestParts<'a> for PathBorrowed<'a> {
  type Error = Infallible;

  fn from_request_parts(
    parts: &'a mut http::request::Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    ready(Ok(PathBorrowed(parts.uri.path())))
  }
}
