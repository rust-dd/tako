//! The query string, raw or deserialized into a `T` that borrows from the URI.

use std::convert::Infallible;
use std::future::ready;

use http::StatusCode;
use http::request::Parts;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;

/// The raw query string; empty when there is none.
pub struct RawQueryBorrowed<'a>(pub &'a str);

impl<'a> FromRequest<'a> for RawQueryBorrowed<'a> {
  type Error = Infallible;

  fn from_request(
    req: &'a mut tako_rs_core::types::Request,
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

/// The query string as a `T` that may borrow from the URI.
pub struct QueryBorrowed<'a, T>(pub T, std::marker::PhantomData<&'a ()>);

/// Rejection of [`QueryBorrowed`]; responds 400.
#[derive(Debug)]
pub enum QueryBorrowedError {
  DeserializationError(String),
}

impl Responder for QueryBorrowedError {
  fn into_response(self) -> tako_rs_core::types::Response {
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
    req: &'a mut tako_rs_core::types::Request,
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
