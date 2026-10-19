//! `ContentLengthLimit<T, N>`, a body-size bound around another extractor.

use http::StatusCode;
use http_body_util::Limited;
use tako_rs_core::body::TakoBody;
use tako_rs_core::extractors::Entries;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;

/// Rejects a declared `Content-Length` above `N` with 413 before reading, then runs `T` on a
/// body capped at `N` bytes, which also bounds chunked bodies and a lying header.
pub struct ContentLengthLimit<T, const N: u64>(pub T);

/// Rejection of [`ContentLengthLimit`].
#[derive(Debug)]
pub enum ContentLengthLimitError<E> {
  TooLarge {
    declared: u64,
    limit: u64,
  },
  /// The `Content-Length` header is not a number; responds 400.
  Malformed,
  Inner(E),
}

impl<E> Responder for ContentLengthLimitError<E>
where
  E: Responder,
{
  fn into_response(self) -> tako_rs_core::types::Response {
    match self {
      Self::TooLarge { declared, limit } => (
        StatusCode::PAYLOAD_TOO_LARGE,
        format!("payload too large: declared {declared} bytes, limit {limit} bytes"),
      )
        .into_response(),
      Self::Malformed => {
        (StatusCode::BAD_REQUEST, "malformed Content-Length header").into_response()
      }
      Self::Inner(e) => e.into_response(),
    }
  }
}

fn check_limit<E>(headers: &http::HeaderMap, limit: u64) -> Result<(), ContentLengthLimitError<E>> {
  let Some(raw) = headers.get(http::header::CONTENT_LENGTH) else {
    return Ok(());
  };
  let declared: u64 = raw
    .to_str()
    .ok()
    .and_then(|s| s.trim().parse().ok())
    .ok_or(ContentLengthLimitError::Malformed)?;
  if declared > limit {
    return Err(ContentLengthLimitError::TooLarge { declared, limit });
  }
  Ok(())
}

impl<'a, T, const N: u64> FromRequest<'a> for ContentLengthLimit<T, N>
where
  T: FromRequest<'a> + Send + 'a,
{
  type Error = ContentLengthLimitError<T::Error>;
  const ENTRIES: Entries = <T as FromRequest<'a>>::ENTRIES;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      check_limit::<T::Error>(req.headers(), N)?;
      let limit = usize::try_from(N).unwrap_or(usize::MAX);
      let body = std::mem::take(req.body_mut());
      *req.body_mut() = TakoBody::new(Limited::new(body, limit));
      T::from_request(req)
        .await
        .map(ContentLengthLimit)
        .map_err(ContentLengthLimitError::Inner)
    }
  }
}
