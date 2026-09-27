//! Bounded buffering shared by body-consuming extractors.

use bytes::Bytes;
use http::StatusCode;
use http_body_util::BodyExt;
use http_body_util::LengthLimitError;
use http_body_util::Limited;

use super::FromRequest;
use crate::responder::Responder;
use crate::types::Request;
use crate::types::Response;

/// Maximum buffered request size; `None` explicitly disables the limit.
#[derive(Clone, Copy, Debug)]
pub struct BodyLimit(pub Option<usize>);

impl Default for BodyLimit {
  fn default() -> Self {
    Self(Some(2 * 1024 * 1024))
  }
}

/// A typed failure while buffering a request body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BodyReadError {
  /// The request exceeds the configured buffering limit.
  TooLarge,
  /// The transport could not supply the body.
  Read(String),
  /// A text extractor received invalid UTF-8.
  InvalidUtf8,
}

impl BodyReadError {
  /// HTTP status associated with this rejection.
  pub fn status(&self) -> StatusCode {
    match self {
      Self::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
      Self::Read(_) | Self::InvalidUtf8 => StatusCode::BAD_REQUEST,
    }
  }

  fn from_boxed(error: crate::types::BoxError) -> Self {
    let mut source = Some(error.as_ref() as &(dyn std::error::Error + 'static));
    while let Some(error) = source {
      if error.is::<LengthLimitError>() {
        return Self::TooLarge;
      }
      source = error.source();
    }
    Self::Read(error.to_string())
  }
}

impl std::fmt::Display for BodyReadError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::TooLarge => f.write_str("request body exceeds the configured limit"),
      Self::Read(error) => write!(f, "failed to read request body: {error}"),
      Self::InvalidUtf8 => f.write_str("request body is not valid UTF-8"),
    }
  }
}

impl std::error::Error for BodyReadError {}

impl Responder for BodyReadError {
  fn into_response(self) -> Response {
    (self.status(), self.to_string()).into_response()
  }
}

/// Buffers a body under the router's limit, including chunked requests.
pub async fn collect_body(req: &mut Request) -> Result<Bytes, BodyReadError> {
  let limit = req
    .extensions()
    .get::<BodyLimit>()
    .copied()
    .unwrap_or_default();
  let collected = match limit.0 {
    Some(limit) => Limited::new(req.body_mut(), limit).collect().await,
    None => req.body_mut().collect().await,
  };
  collected
    .map(http_body_util::Collected::to_bytes)
    .map_err(BodyReadError::from_boxed)
}

impl<'a> FromRequest<'a> for Bytes {
  type Error = BodyReadError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl Future<Output = Result<Self, Self::Error>> + Send + 'a {
    collect_body(req)
  }
}

impl<'a> FromRequest<'a> for String {
  type Error = BodyReadError;

  async fn from_request(req: &'a mut Request) -> Result<Self, Self::Error> {
    String::from_utf8(collect_body(req).await?.to_vec()).map_err(|_| BodyReadError::InvalidUtf8)
  }
}
