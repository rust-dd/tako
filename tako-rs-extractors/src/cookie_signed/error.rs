use http::StatusCode;
use tako_rs_core::responder::Responder;

/// Rejection of [`CookieSigned`](crate::cookie_signed::CookieSigned).
#[derive(Debug)]
pub enum CookieSignedError {
  /// Neither a `KeyRing` nor a `Key` is in the request extensions; responds 500.
  MissingKey,
}

impl Responder for CookieSignedError {
  fn into_response(self) -> tako_rs_core::types::Response {
    match self {
      CookieSignedError::MissingKey => (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Signed cookie master key not found in request extensions",
      )
        .into_response(),
    }
  }
}
