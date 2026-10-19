//! Path parameters parsed field by field with `FromStr` into the struct `#[tako::route]`
//! generates (`get_user` → `GetUserParams`); errors name the offending field.

use core::fmt;

use http::StatusCode;
use http::request::Parts;

use crate::extractors::Entries;
use crate::extractors::FromRequest;
use crate::extractors::FromRequestParts;
use crate::extractors::params::PathParams;
use crate::responder::Responder;
use crate::types::Request;
use crate::types::Response;

/// Trait implemented (via `#[tako::route]`) by structs that mirror a route's
/// path placeholders. Hand-written impls are also supported.
pub trait TypedParamsStruct: Sized {
  /// Builds `Self` from the request's [`PathParams`].
  fn from_path_params(pp: &PathParams) -> Result<Self, TypedParamsError>;
}

/// Extractor wrapper that delegates to [`TypedParamsStruct::from_path_params`].
pub struct TypedParams<T>(pub T);

/// Errors produced by [`TypedParams`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypedParamsError {
  /// The router never inserted `PathParams` (no dynamic segments matched).
  Missing,
  /// A placeholder declared by the typed struct was absent from the matched
  /// route. Usually indicates a typo in a hand-written `impl TypedParamsStruct`.
  MissingField(&'static str),
  /// The placeholder's raw value failed `FromStr` parsing.
  Parse(&'static str, String),
}

impl fmt::Display for TypedParamsError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Missing => f.write_str("path parameters not found in request extensions"),
      Self::MissingField(name) => write!(f, "missing path param '{name}'"),
      Self::Parse(name, err) => write!(f, "invalid path param '{name}': {err}"),
    }
  }
}

impl std::error::Error for TypedParamsError {}

impl Responder for TypedParamsError {
  fn into_response(self) -> Response {
    match self {
      Self::Missing => (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Path parameters not found in request extensions",
      )
        .into_response(),
      Self::MissingField(_) | Self::Parse(_, _) => {
        (StatusCode::BAD_REQUEST, self.to_string()).into_response()
      }
    }
  }
}

impl<'a, T> FromRequestParts<'a> for TypedParams<T>
where
  T: TypedParamsStruct + Send + 'a,
{
  type Error = TypedParamsError;
  const ENTRIES: Entries = Entries::PARAMS;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = Result<Self, Self::Error>> + Send + 'a {
    let result: Result<Self, Self::Error> = match parts.extensions.get::<PathParams>() {
      Some(pp) => T::from_path_params(pp).map(TypedParams),
      None => Err(TypedParamsError::Missing),
    };
    futures_util::future::ready(result)
  }
}

impl<'a, T> FromRequest<'a> for TypedParams<T>
where
  T: TypedParamsStruct + Send + 'a,
{
  type Error = TypedParamsError;
  const ENTRIES: Entries = Entries::PARAMS;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = Result<Self, Self::Error>> + Send + 'a {
    let result: Result<Self, Self::Error> = match req.extensions().get::<PathParams>() {
      Some(pp) => T::from_path_params(pp).map(TypedParams),
      None => Err(TypedParamsError::Missing),
    };
    futures_util::future::ready(result)
  }
}
