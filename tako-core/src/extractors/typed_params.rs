//! Compile-time-shaped path parameter extractor backed by [`FromStr`].
//!
//! Pair this with the [`#[tako::route]`](https://docs.rs/tako-rs) attribute
//! macro from `tako-macros`: place the attribute on an async handler and the
//! macro generates a sibling struct whose fields exactly mirror the path
//! placeholders, associated `METHOD`/`PATH` consts, and a
//! [`TypedParamsStruct`](crate::extractors::typed_params::TypedParamsStruct) impl that pulls each value from the request's
//! [`PathParams`] extension and parses it via [`core::str::FromStr`]. The
//! struct name defaults to the handler's name in PascalCase plus a `Params`
//! suffix (e.g. `get_user` → `GetUserParams`).
//!
//! Compared to [`Params<T>`](crate::extractors::params::Params), this extractor
//! sidesteps `serde::Deserialize` entirely — the generated impl is direct
//! field-by-field parsing, and parse errors carry the offending field name in
//! the response body.
//!
//! [`PathParams`]: crate::extractors::params::PathParams
//! [`FromStr`]: core::str::FromStr

use core::fmt;

use http::StatusCode;
use http::request::Parts;

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
