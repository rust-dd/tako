use http::StatusCode;
use tako_rs_core::responder::Responder;

/// Error type for signed cookie extraction.
#[derive(Debug)]
pub enum CookieSignedError {
  /// Signed cookie master key not found in request extensions.
  MissingKey,
  /// Invalid signed cookie master key.
  InvalidKey,
  /// Failed to verify signed cookie with the specified error message.
  VerificationFailed(String),
  /// Invalid cookie format in request.
  InvalidCookieFormat,
  /// Invalid signature for the specified cookie name.
  InvalidSignature(String),
}

impl Responder for CookieSignedError {
  /// Converts the error into an HTTP response.
  fn into_response(self) -> tako_rs_core::types::Response {
    match self {
      CookieSignedError::MissingKey => (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Signed cookie master key not found in request extensions",
      )
        .into_response(),
      CookieSignedError::InvalidKey => (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Invalid signed cookie master key",
      )
        .into_response(),
      CookieSignedError::VerificationFailed(err) => (
        StatusCode::BAD_REQUEST,
        format!("Failed to verify signed cookie: {err}"),
      )
        .into_response(),
      CookieSignedError::InvalidCookieFormat => {
        (StatusCode::BAD_REQUEST, "Invalid cookie format in request").into_response()
      }
      CookieSignedError::InvalidSignature(cookie_name) => (
        StatusCode::BAD_REQUEST,
        format!("Invalid signature for cookie: {cookie_name}"),
      )
        .into_response(),
    }
  }
}
