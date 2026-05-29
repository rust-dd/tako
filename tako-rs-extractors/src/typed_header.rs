#![cfg_attr(docsrs, doc(cfg(feature = "typed-header")))]

//! `TypedHeader<H>` extractor — axum parity built on the `headers` crate.
//!
//! Decodes a single HTTP header into a strongly-typed value via the
//! `headers::Header` trait, returning a 400 `Responder` rejection on missing
//! or malformed input.
//!
//! Enable with the `typed-header` cargo feature.
//!
//! # Examples
//!
//! ```rust,ignore
//! use tako::extractors::typed_header::TypedHeader;
//! use headers::UserAgent;
//!
//! async fn handler(TypedHeader(ua): TypedHeader<UserAgent>) -> String {
//!   format!("ua = {ua}")
//! }
//! ```
//!
//! Optional headers can be obtained via `Option<TypedHeader<H>>` because
//! `Option<E>` is supported by the handler machinery.

use http::StatusCode;
use http::request::Parts;
use tako_core::extractors::FromRequest;
use tako_core::extractors::FromRequestParts;
use tako_core::responder::Responder;
use tako_core::types::Request;

/// A strongly-typed header extractor.
///
/// `H` must implement `headers::Header` (e.g. `headers::UserAgent`,
/// `headers::ContentType`, `headers::Authorization<Bearer>`).
pub struct TypedHeader<H>(pub H);

/// Rejection produced when `TypedHeader<H>` cannot extract its value.
#[derive(Debug)]
pub enum TypedHeaderRejection {
  /// Header was absent from the request.
  Missing(&'static str),
  /// Header was present but failed to decode into `H`.
  Invalid {
    /// Header name.
    name: &'static str,
    /// Underlying decode error message.
    error: String,
  },
}

impl Responder for TypedHeaderRejection {
  fn into_response(self) -> tako_core::types::Response {
    match self {
      TypedHeaderRejection::Missing(name) => (
        StatusCode::BAD_REQUEST,
        format!("missing required header: {name}"),
      )
        .into_response(),
      TypedHeaderRejection::Invalid { name, error } => (
        StatusCode::BAD_REQUEST,
        format!("invalid header `{name}`: {error}"),
      )
        .into_response(),
    }
  }
}

fn decode<H>(headers: &http::HeaderMap) -> Result<H, TypedHeaderRejection>
where
  H: headers::Header,
{
  let name = H::name().as_str();
  let mut iter = headers.get_all(H::name()).iter();
  if iter.size_hint().1 == Some(0) && headers.get(H::name()).is_none() {
    return Err(TypedHeaderRejection::Missing(static_name::<H>(name)));
  }
  H::decode(&mut iter).map_err(|e| TypedHeaderRejection::Invalid {
    name: static_name::<H>(name),
    error: e.to_string(),
  })
}

// `headers::Header::name()` returns a `&'static HeaderName`, but we want a
// `&'static str` for the rejection message. Both representations live for
// 'static, so a transmute-free path through `.as_str()` is sufficient — the
// indirection here just narrows the type.
fn static_name<H: headers::Header>(_runtime: &str) -> &'static str {
  H::name().as_str()
}

impl<'a, H> FromRequest<'a> for TypedHeader<H>
where
  H: headers::Header + Send + 'a,
{
  type Error = TypedHeaderRejection;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(decode::<H>(req.headers()).map(TypedHeader))
  }
}

impl<'a, H> FromRequestParts<'a> for TypedHeader<H>
where
  H: headers::Header + Send + 'a,
{
  type Error = TypedHeaderRejection;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(decode::<H>(&parts.headers).map(TypedHeader))
  }
}
