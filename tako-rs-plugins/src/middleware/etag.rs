//! Weak SHA-256 `ETag`s for 200 responses to GET and HEAD, answering matching conditionals
//! with 304. Bodies are buffered to hash them, so skip it on streaming routes.

use std::future::Future;
use std::pin::Pin;

use http::HeaderValue;
use http::Method;
use http::StatusCode;
use http::header::CONTENT_LENGTH;
use http::header::ETAG;
use http::header::IF_MODIFIED_SINCE;
use http::header::IF_NONE_MATCH;
use http::header::LAST_MODIFIED;
use http_body::Body;
use http_body_util::BodyExt;
use sha2::Digest;
use sha2::Sha256;
use tako_rs_core::body::TakoBody;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

pub struct ETag {
  max_bytes: usize,
}

impl Default for ETag {
  fn default() -> Self {
    Self::new()
  }
}

impl ETag {
  /// Creates the middleware with a 1 MiB body cap.
  pub fn new() -> Self {
    Self {
      max_bytes: 1024 * 1024,
    }
  }

  /// Larger bodies of known size pass through; a streamed body that exceeds it becomes a 500.
  pub fn max_bytes(mut self, n: usize) -> Self {
    self.max_bytes = n;
    self
  }
}

fn weak_match(if_none_match: &str, etag: &str) -> bool {
  // Both `*` (wildcard) and a comma-separated list are valid.
  if if_none_match.trim() == "*" {
    return true;
  }
  if_none_match.split(',').any(|raw| {
    let raw = raw.trim();
    let candidate = raw.strip_prefix("W/").unwrap_or(raw);
    let etag_norm = etag.strip_prefix("W/").unwrap_or(etag);
    candidate == etag_norm
  })
}

fn build_304(mut status_headers: http::HeaderMap) -> Response {
  let mut resp = http::Response::builder()
    .status(StatusCode::NOT_MODIFIED)
    .body(TakoBody::empty())
    .expect("valid 304 response");
  status_headers.remove(CONTENT_LENGTH);
  *resp.headers_mut() = status_headers;
  resp
}

/// SHA-256 so colliding bodies cannot share a validator; weak because negotiated metadata
/// such as the content type is not hashed.
fn make_etag(bytes: &[u8]) -> String {
  let mut hasher = Sha256::new();
  hasher.update(bytes);
  let digest = hasher.finalize();
  let mut hex = String::with_capacity(4 + 64);
  hex.push_str("W/\"");
  for b in digest {
    use std::fmt::Write;
    let _ = write!(hex, "{b:02x}");
  }
  hex.push('"');
  hex
}

/// Compares parsed dates: RFC 9110 §5.6.7 allows IMF-fixdate, RFC 850 and asctime forms.
fn not_modified_since(if_modified_since: &str, last_modified: &str) -> bool {
  match (
    httpdate::parse_http_date(if_modified_since.trim()),
    httpdate::parse_http_date(last_modified.trim()),
  ) {
    (Ok(ims), Ok(lm)) => ims >= lm,
    _ => false,
  }
}

impl IntoMiddleware for ETag {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let max_bytes = self.max_bytes;

    move |req: Request, next: Next| {
      Box::pin(async move {
        let safe = matches!(*req.method(), Method::GET | Method::HEAD);
        let if_none_match = req
          .headers()
          .get(IF_NONE_MATCH)
          .and_then(|v| v.to_str().ok())
          .map(str::to_string);
        let if_modified_since = if req.headers().contains_key(IF_NONE_MATCH) {
          None
        } else {
          let mut values = req.headers().get_all(IF_MODIFIED_SINCE).iter();
          values
            .next()
            .filter(|_| values.next().is_none())
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
        };

        let resp = next.run(req).await;
        if !safe || resp.status() != StatusCode::OK {
          return resp;
        }

        if let Some(ims) = if_modified_since.as_ref()
          && let Some(lm) = resp
            .headers()
            .get(LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
          && not_modified_since(ims, lm)
        {
          return build_304(resp.into_parts().0.headers);
        }

        if let Some(existing_etag) = resp
          .headers()
          .get(ETAG)
          .and_then(|v| v.to_str().ok())
          .map(str::to_string)
        {
          if let Some(req_etag) = if_none_match.as_ref()
            && weak_match(req_etag, &existing_etag)
          {
            return build_304(resp.into_parts().0.headers);
          }
          return resp;
        }

        // `Limited` also caps bodies that under-report their size.
        let (parts, body) = resp.into_parts();
        if let Some(n) = body.size_hint().exact()
          && (n as usize) > max_bytes
        {
          return http::Response::from_parts(parts, body);
        }
        let limited = http_body_util::Limited::new(body, max_bytes);
        let collected = match limited.collect().await {
          Ok(c) => c.to_bytes(),
          Err(_) => {
            // Partially consumed, so the original body cannot be replayed.
            return http::Response::builder()
              .status(StatusCode::INTERNAL_SERVER_ERROR)
              .body(TakoBody::empty())
              .expect("valid 500 response");
          }
        };
        let etag = make_etag(&collected);
        let mut resp = http::Response::from_parts(parts, TakoBody::from(collected));
        if let Ok(v) = HeaderValue::from_str(&etag) {
          resp.headers_mut().insert(ETAG, v);
        }
        if let Some(req_etag) = if_none_match.as_ref()
          && weak_match(req_etag, &etag)
        {
          return build_304(resp.into_parts().0.headers);
        }
        resp
      })
    }
  }
}
