//! `MatchedPath` extractor — the route template that matched the request.
//!
//! Returns the routing template (e.g. `/users/{id}`) instead of the concrete
//! URI (`/users/42`). Useful for metrics labels (avoiding cardinality blow-ups)
//! and structured logs.
//!
//! The router inserts `tako_core::router_state::MatchedPath` into request
//! extensions during dispatch; this extractor surfaces it.

use http::StatusCode;
use http::request::Parts;
use tako_core::extractors::FromRequest;
use tako_core::extractors::FromRequestParts;
use tako_core::responder::Responder;
use tako_core::router_state::MatchedPath as MatchedPathExt;
use tako_core::types::Request;

/// Owned-string view of the matched route template.
pub struct MatchedPath(pub String);

/// Rejection when no `MatchedPath` extension is on the request.
#[derive(Debug)]
pub struct MatchedPathMissing;

impl Responder for MatchedPathMissing {
  fn into_response(self) -> tako_core::types::Response {
    (
      StatusCode::INTERNAL_SERVER_ERROR,
      "matched path is unavailable on this request",
    )
      .into_response()
  }
}

impl<'a> FromRequest<'a> for MatchedPath {
  type Error = MatchedPathMissing;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(
      req
        .extensions()
        .get::<MatchedPathExt>()
        .map(|m| MatchedPath(m.0.clone()))
        .ok_or(MatchedPathMissing),
    )
  }
}

impl<'a> FromRequestParts<'a> for MatchedPath {
  type Error = MatchedPathMissing;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(
      parts
        .extensions
        .get::<MatchedPathExt>()
        .map(|m| MatchedPath(m.0.clone()))
        .ok_or(MatchedPathMissing),
    )
  }
}
