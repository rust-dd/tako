//! Verified JWT claims extractor.
//!
//! Pairs with [`crate::middleware::jwt_auth::JwtAuth<V>`]: when the middleware
//! runs successfully, it inserts the decoded `V::Claims` into request
//! extensions. `JwtClaimsVerified<C>` extracts that value, returning a clear
//! 401 rejection if the middleware did not run (a typical wiring mistake).
//!
//! For unauthenticated decoding (no signature check) prefer
//! `tako_extractors::jwt::JwtClaimsUnverified<T>`.

use http::StatusCode;
use http::request::Parts;
use tako_core::extractors::FromRequest;
use tako_core::extractors::FromRequestParts;
use tako_core::responder::Responder;
use tako_core::types::Request;

/// Verified JWT claims placed into request extensions by [`crate::middleware::jwt_auth::JwtAuth`].
///
/// `C` must be the verifier's `Claims` type (not the raw JWT payload).
pub struct JwtClaimsVerified<C>(pub C);

/// Rejection when the auth middleware did not run for this request.
#[derive(Debug)]
pub struct UnverifiedClaims;

impl Responder for UnverifiedClaims {
  fn into_response(self) -> tako_core::types::Response {
    (
      StatusCode::UNAUTHORIZED,
      "request was not authenticated by JwtAuth middleware",
    )
      .into_response()
  }
}

impl<'a, C> FromRequest<'a> for JwtClaimsVerified<C>
where
  C: Clone + Send + Sync + 'static,
{
  type Error = UnverifiedClaims;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(
      req
        .extensions()
        .get::<C>()
        .cloned()
        .map(JwtClaimsVerified)
        .ok_or(UnverifiedClaims),
    )
  }
}

impl<'a, C> FromRequestParts<'a> for JwtClaimsVerified<C>
where
  C: Clone + Send + Sync + 'static,
{
  type Error = UnverifiedClaims;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(
      parts
        .extensions
        .get::<C>()
        .cloned()
        .map(JwtClaimsVerified)
        .ok_or(UnverifiedClaims),
    )
  }
}
