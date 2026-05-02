//! `Extension<T>` extractor — axum parity for request-scoped values.
//!
//! Pulls a clone of `T` out of `request.extensions()`. Middleware that wants
//! to expose typed data to a handler can `req.extensions_mut().insert(value)`
//! and the handler reaches it via `Extension<MyType>`.
//!
//! # Examples
//!
//! ```rust,ignore
//! use tako::extractors::extension::Extension;
//!
//! #[derive(Clone)]
//! struct CurrentUser { id: u64 }
//!
//! async fn handler(Extension(user): Extension<CurrentUser>) -> String {
//!   format!("user = {}", user.id)
//! }
//! ```

use http::StatusCode;
use http::request::Parts;
use tako_core::extractors::FromRequest;
use tako_core::extractors::FromRequestParts;
use tako_core::responder::Responder;
use tako_core::types::Request;

/// Extracts a clone of a typed extension value.
pub struct Extension<T>(pub T);

/// Rejection when no value of type `T` is present in extensions.
#[derive(Debug)]
pub struct MissingExtension(pub &'static str);

impl Responder for MissingExtension {
  fn into_response(self) -> tako_core::types::Response {
    (
      StatusCode::INTERNAL_SERVER_ERROR,
      format!("missing extension: {}", self.0),
    )
      .into_response()
  }
}

impl<'a, T> FromRequest<'a> for Extension<T>
where
  T: Clone + Send + Sync + 'static,
{
  type Error = MissingExtension;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(
      req
        .extensions()
        .get::<T>()
        .cloned()
        .map(Extension)
        .ok_or(MissingExtension(std::any::type_name::<T>())),
    )
  }
}

impl<'a, T> FromRequestParts<'a> for Extension<T>
where
  T: Clone + Send + Sync + 'static,
{
  type Error = MissingExtension;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(
      parts
        .extensions
        .get::<T>()
        .cloned()
        .map(Extension)
        .ok_or(MissingExtension(std::any::type_name::<T>())),
    )
  }
}
