//! Claims verified by [`JwtAuth`](crate::middleware::jwt_auth::JwtAuth); extraction fails with
//! 401 when that middleware did not run.

use http::StatusCode;
use http::request::Parts;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;

/// `C` must be the verifier's `Claims` type, not the raw JWT payload.
pub struct JwtClaimsVerified<C>(pub C);

/// Rejection when the auth middleware did not run for this request.
#[derive(Debug)]
pub struct UnverifiedClaims;

impl Responder for UnverifiedClaims {
  fn into_response(self) -> tako_rs_core::types::Response {
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
