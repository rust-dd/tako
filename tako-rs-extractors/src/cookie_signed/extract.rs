use cookie::Key;
use http::request::Parts;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::types::Request;

use crate::cookie_signed::CookieSigned;
use crate::cookie_signed::CookieSignedError;
use crate::cookie_signed::KeyRing;

impl CookieSigned {
  /// Extracts signed cookies from a request, preferring a [`KeyRing`] over a
  /// single [`Key`] when both are present in extensions.
  fn extract_from_request(req: &Request) -> Result<Self, CookieSignedError> {
    if let Some(ring) = req.extensions().get::<KeyRing>().cloned() {
      return Ok(Self::from_headers_with_ring(req.headers(), ring));
    }
    let key = req
      .extensions()
      .get::<Key>()
      .ok_or(CookieSignedError::MissingKey)?
      .clone();
    Ok(Self::from_headers(req.headers(), key))
  }

  /// Same as [`Self::extract_from_request`] but for `Parts`.
  fn extract_from_parts(parts: &Parts) -> Result<Self, CookieSignedError> {
    if let Some(ring) = parts.extensions.get::<KeyRing>().cloned() {
      return Ok(Self::from_headers_with_ring(&parts.headers, ring));
    }
    let key = parts
      .extensions
      .get::<Key>()
      .ok_or(CookieSignedError::MissingKey)?
      .clone();
    Ok(Self::from_headers(&parts.headers, key))
  }
}

impl<'a> FromRequest<'a> for CookieSigned {
  type Error = CookieSignedError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from_request(req))
  }
}

impl<'a> FromRequestParts<'a> for CookieSigned {
  type Error = CookieSignedError;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from_parts(parts))
  }
}
