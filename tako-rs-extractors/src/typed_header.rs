//! `TypedHeader<H>`: one header decoded through `headers::Header`; missing or malformed is 400.
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

use http::StatusCode;
use http::request::Parts;
use tako_rs_core::extractors::Entries;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;

pub struct TypedHeader<H>(pub H);

/// Rejection of [`TypedHeader`], carrying the header name.
#[derive(Debug)]
pub enum TypedHeaderRejection {
  Missing(&'static str),
  Invalid { name: &'static str, error: String },
}

impl Responder for TypedHeaderRejection {
  fn into_response(self) -> tako_rs_core::types::Response {
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
  if !headers.contains_key(H::name()) {
    return Err(TypedHeaderRejection::Missing(name));
  }
  H::decode(&mut headers.get_all(H::name()).iter()).map_err(|e| TypedHeaderRejection::Invalid {
    name,
    error: e.to_string(),
  })
}

impl<'a, H> FromRequest<'a> for TypedHeader<H>
where
  H: headers::Header + Send + 'a,
{
  type Error = TypedHeaderRejection;
  const ENTRIES: Entries = Entries::NONE;

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
  const ENTRIES: Entries = Entries::NONE;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(decode::<H>(&parts.headers).map(TypedHeader))
  }
}
