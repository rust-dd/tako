//! Zero-copy single-header extractor.

use http::HeaderName;
use http::HeaderValue;
use http::StatusCode;
use http::request::Parts;
use tako_core::extractors::FromRequest;
use tako_core::extractors::FromRequestParts;
use tako_core::responder::Responder;

/// Borrowed reference to a single header value, addressed by a const name.
///
/// The const-generic `NAME` pattern means the header name is encoded into the
/// type — e.g. `HeaderBorrowed<{ "x-trace-id" }>` — but Rust does not yet allow
/// `&'static str` as a const-generic. Instead we expose `HeaderRef<'a>` plus
/// the helper `HeaderBorrowedBy<'a, const N: usize>` for static byte names.
pub struct HeaderRef<'a> {
  /// The header name that was looked up.
  pub name: &'static str,
  /// The borrowed value, if present.
  pub value: Option<&'a HeaderValue>,
}

/// Builder helper that resolves a single header name lazily.
///
/// Use `BorrowedHeader::new("x-request-id").extract(parts)` from inside a
/// custom extractor. For routing-time fixed names prefer `HeaderRequired`.
pub struct BorrowedHeader {
  name: HeaderName,
}

impl BorrowedHeader {
  /// Build a borrowed-header lookup for `name`.
  pub fn new(name: &'static str) -> Self {
    Self {
      name: HeaderName::from_static(name),
    }
  }

  /// Borrowed lookup against a header map.
  pub fn lookup<'a>(&self, headers: &'a http::HeaderMap) -> Option<&'a HeaderValue> {
    headers.get(&self.name)
  }
}

/// Required-header zero-copy extractor.
///
/// Concrete handlers use this through `tako::extractors::header_required!`
/// — see the macro in `tako-extractors` if/when it lands. Until then this
/// type is a building block: implement `FromRequest`/`FromRequestParts` on a
/// newtype that calls `BorrowedHeader::new(...)` and either returns the value
/// or a `MissingHeader` rejection.
pub struct MissingHeader(pub &'static str);

impl Responder for MissingHeader {
  fn into_response(self) -> tako_core::types::Response {
    (
      StatusCode::BAD_REQUEST,
      format!("missing required header: {}", self.0),
    )
      .into_response()
  }
}

/// Concrete `Authorization` zero-copy required-header extractor.
///
/// Demonstrates the pattern; downstream crates can copy/paste for their own
/// header-bound newtypes without inventing a new const-generic system.
pub struct AuthorizationBorrowed<'a>(pub &'a HeaderValue);

impl<'a> FromRequest<'a> for AuthorizationBorrowed<'a> {
  type Error = MissingHeader;

  fn from_request(
    req: &'a mut tako_core::types::Request,
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

/// Optional `Authorization` zero-copy extractor — `None` if the header is absent.
pub struct AuthorizationOptBorrowed<'a>(pub Option<&'a HeaderValue>);

impl<'a> FromRequest<'a> for AuthorizationOptBorrowed<'a> {
  type Error = std::convert::Infallible;

  fn from_request(
    req: &'a mut tako_core::types::Request,
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
  /// Convenience to fold a `HeaderRef` into a string slice when the value is UTF-8.
  pub fn as_str(&self) -> Option<&str> {
    self.value.and_then(|v| v.to_str().ok())
  }
}
