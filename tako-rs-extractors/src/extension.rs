//! `Extension<T>`: a clone of the `T` that middleware put into the request extensions.
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
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;

pub struct Extension<T>(pub T);

/// No `T` in the extensions. Holds the full [`std::any::type_name`]; the 500 response names
/// only [`Self::short_name`].
#[derive(Debug)]
pub struct MissingExtension(pub &'static str);

impl MissingExtension {
  /// The type name without its module path; generic arguments keep theirs.
  pub fn short_name(&self) -> &'static str {
    let s: &'static str = self.0;
    let cutoff = s.find('<').unwrap_or(s.len());
    match s[..cutoff].rfind("::") {
      Some(idx) => &s[idx + 2..],
      None => s,
    }
  }
}

impl Responder for MissingExtension {
  fn into_response(self) -> tako_rs_core::types::Response {
    // The full path would leak crate structure to clients.
    let short = self.short_name();
    (
      StatusCode::INTERNAL_SERVER_ERROR,
      format!("missing extension: {short}"),
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
