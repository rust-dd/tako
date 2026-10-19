//! Zero-copy single-header extractor.

use http::HeaderName;
use http::HeaderValue;
use http::StatusCode;
use http::request::Parts;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;

/// A header name and its borrowed value, if present.
pub struct HeaderRef<'a> {
  pub name: &'static str,
  pub value: Option<&'a HeaderValue>,
}

/// A header name parsed once for lookups in custom extractors; `new` panics on a name that is
/// not lowercase and valid.
pub struct BorrowedHeader {
  name: HeaderName,
}

impl BorrowedHeader {
  pub fn new(name: &'static str) -> Self {
    Self {
      name: HeaderName::from_static(name),
    }
  }

  pub fn lookup<'a>(&self, headers: &'a http::HeaderMap) -> Option<&'a HeaderValue> {
    headers.get(&self.name)
  }
}

/// A required header is absent; responds 400.
pub struct MissingHeader(pub &'static str);

impl Responder for MissingHeader {
  fn into_response(self) -> tako_rs_core::types::Response {
    (
      StatusCode::BAD_REQUEST,
      format!("missing required header: {}", self.0),
    )
      .into_response()
  }
}

/// The `Authorization` header value; rejects with [`MissingHeader`] when absent.
pub struct AuthorizationBorrowed<'a>(pub &'a HeaderValue);

impl<'a> FromRequest<'a> for AuthorizationBorrowed<'a> {
  type Error = MissingHeader;

  fn from_request(
    req: &'a mut tako_rs_core::types::Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(
      req
        .headers()
        .get(http::header::AUTHORIZATION)
        .map(AuthorizationBorrowed)
        .ok_or(MissingHeader("authorization")),
    )
  }
}

impl<'a> FromRequestParts<'a> for AuthorizationBorrowed<'a> {
  type Error = MissingHeader;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(
      parts
        .headers
        .get(http::header::AUTHORIZATION)
        .map(AuthorizationBorrowed)
        .ok_or(MissingHeader("authorization")),
    )
  }
}

pub struct AuthorizationOptBorrowed<'a>(pub Option<&'a HeaderValue>);

impl<'a> FromRequest<'a> for AuthorizationOptBorrowed<'a> {
  type Error = std::convert::Infallible;

  fn from_request(
    req: &'a mut tako_rs_core::types::Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Ok(AuthorizationOptBorrowed(
      req.headers().get(http::header::AUTHORIZATION),
    )))
  }
}

impl<'a> FromRequestParts<'a> for AuthorizationOptBorrowed<'a> {
  type Error = std::convert::Infallible;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Ok(AuthorizationOptBorrowed(
      parts.headers.get(http::header::AUTHORIZATION),
    )))
  }
}

impl HeaderRef<'_> {
  /// The value, when present and visible ASCII.
  pub fn as_str(&self) -> Option<&str> {
    self.value.and_then(|v| v.to_str().ok())
  }
}
