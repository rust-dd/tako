//! Zero-copy query-string extractor.
//!
//! `QueryBorrowed<'a, T>` deserializes from the URI query slice directly. The
//! borrowed `T` (e.g. `Cow<'a, str>` fields) lives as long as the request, so
//! string parameters do not need to be cloned out of the URI buffer.

use std::convert::Infallible;
use std::future::ready;

use http::StatusCode;
use http::request::Parts;
use tako_core::extractors::FromRequest;
use tako_core::extractors::FromRequestParts;
use tako_core::responder::Responder;

/// Zero-copy access to the raw query string.
pub struct RawQueryBorrowed<'a>(pub &'a str);

impl<'a> FromRequest<'a> for RawQueryBorrowed<'a> {
  type Error = Infallible;

  fn from_request(
    req: &'a mut tako_core::types::Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    ready(Ok(RawQueryBorrowed(req.uri().query().unwrap_or(""))))
  }
}

impl<'a> FromRequestParts<'a> for RawQueryBorrowed<'a> {
  type Error = Infallible;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    ready(Ok(RawQueryBorrowed(parts.uri.query().unwrap_or(""))))
  }
}

/// Zero-copy typed query extractor.
///
/// `T` must be `Deserialize<'a>` so it can borrow from the URI query slice.
pub struct QueryBorrowed<'a, T>(pub T, std::marker::PhantomData<&'a ()>);

/// Error type for `QueryBorrowed`.
#[derive(Debug)]
pub enum QueryBorrowedError {
  /// `serde_urlencoded` failed to deserialize the query slice.
  DeserializationError(String),
}

impl Responder for QueryBorrowedError {
  fn into_response(self) -> tako_core::types::Response {
    match self {
      Self::DeserializationError(e) => (
        StatusCode::BAD_REQUEST,
        format!("failed to deserialize query: {e}"),
      )
        .into_response(),
    }
  }
}

impl<'a, T> FromRequest<'a> for QueryBorrowed<'a, T>
where
  T: serde::Deserialize<'a> + Send + 'a,
{
  type Error = QueryBorrowedError;

  fn from_request(
    req: &'a mut tako_core::types::Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    let q = req.uri().query().unwrap_or("");
    ready(
      serde_urlencoded::from_str::<T>(q)
        .map(|v| QueryBorrowed(v, std::marker::PhantomData))
        .map_err(|e| QueryBorrowedError::DeserializationError(e.to_string())),
    )
  }
}

impl<'a, T> FromRequestParts<'a> for QueryBorrowed<'a, T>
where
  T: serde::Deserialize<'a> + Send + 'a,
{
  type Error = QueryBorrowedError;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    let q = parts.uri.query().unwrap_or("");
    ready(
      serde_urlencoded::from_str::<T>(q)
        .map(|v| QueryBorrowed(v, std::marker::PhantomData))
        .map_err(|e| QueryBorrowedError::DeserializationError(e.to_string())),
    )
  }
}
