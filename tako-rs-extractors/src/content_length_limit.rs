//! `ContentLengthLimit<T, N>` extractor — bounded body extractor wrapper.
//!
//! Performs two layers of defense against payload-bomb DoS:
//!
//! 1. **Header check** — if `Content-Length` is present and exceeds `N`, the
//!    request is rejected with `413 Payload Too Large` before any body bytes
//!    are read.
//! 2. **Streaming cap** — the request body is wrapped in
//!    `http_body_util::Limited<…, N>` so the inner extractor cannot read more
//!    than `N` bytes even when the Content-Length header is missing, lying,
//!    or the transfer is chunked. This blocks the classic
//!    "Content-Length: 0 / Transfer-Encoding: chunked / send forever" attack
//!    that the previous header-only check missed.

use http::StatusCode;
use http_body_util::Limited;
use tako_core::body::TakoBody;
use tako_core::extractors::FromRequest;
use tako_core::responder::Responder;
use tako_core::types::Request;

/// Wraps another extractor and rejects with 413 when `Content-Length` exceeds `N`.
pub struct ContentLengthLimit<T, const N: u64>(pub T);

/// Rejection variants for `ContentLengthLimit`.
#[derive(Debug)]
pub enum ContentLengthLimitError<E> {
  /// Declared `Content-Length` exceeds the limit.
  TooLarge {
    /// Declared length from the header.
    declared: u64,
    /// Configured limit.
    limit: u64,
  },
  /// `Content-Length` was present but failed to parse.
  Malformed,
  /// Inner extractor produced an error.
  Inner(E),
}

impl<E> Responder for ContentLengthLimitError<E>
where
  E: Responder,
{
  fn into_response(self) -> tako_core::types::Response {
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

fn check_limit(headers: &http::HeaderMap, limit: u64) -> Result<(), ContentLengthLimitErrorRaw> {
  let Some(raw) = headers.get(http::header::CONTENT_LENGTH) else {
    // No Content-Length header — leave it to the inner extractor / body limit
    // middleware to enforce streaming caps.
    return Ok(());
  };
  let declared: u64 = raw
    .to_str()
    .ok()
    .and_then(|s| s.trim().parse().ok())
    .ok_or(ContentLengthLimitErrorRaw::Malformed)?;
  if declared > limit {
    return Err(ContentLengthLimitErrorRaw::TooLarge { declared, limit });
  }
  Ok(())
}

#[derive(Debug)]
enum ContentLengthLimitErrorRaw {
  TooLarge { declared: u64, limit: u64 },
  Malformed,
}

impl<'a, T, const N: u64> FromRequest<'a> for ContentLengthLimit<T, N>
where
  T: FromRequest<'a> + Send + 'a,
{
  type Error = ContentLengthLimitError<T::Error>;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      match check_limit(req.headers(), N) {
        Ok(()) => {
          // Wrap the body in `Limited` so the inner extractor cannot read more
          // than N bytes even if Content-Length was missing/false or the
          // transfer is chunked.
          let limit = usize::try_from(N).unwrap_or(usize::MAX);
          let body = std::mem::take(req.body_mut());
          *req.body_mut() = TakoBody::new(Limited::new(body, limit));

          match T::from_request(req).await {
            Ok(v) => Ok(ContentLengthLimit(v)),
            Err(e) => Err(ContentLengthLimitError::Inner(e)),
          }
        }
        Err(ContentLengthLimitErrorRaw::TooLarge { declared, limit }) => {
          Err(ContentLengthLimitError::TooLarge { declared, limit })
        }
        Err(ContentLengthLimitErrorRaw::Malformed) => Err(ContentLengthLimitError::Malformed),
      }
    }
  }
}
