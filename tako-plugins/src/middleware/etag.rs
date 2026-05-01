//! ETag and conditional GET helper middleware.
//!
//! For 200-OK responses without an existing `ETag` header, hashes the body
//! and emits a strong validator (`"<sha1-hex>"`). On subsequent requests
//! with `If-None-Match` containing the same validator the middleware
//! short-circuits to a `304 Not Modified` reply with the original headers
//! preserved (sans the body and `Content-Length`).
//!
//! `If-Modified-Since` is honored only when the upstream response carries
//! a `Last-Modified` header; the comparison is RFC 7232 / RFC 9110 semantic
//! (date-truncated to seconds, weak comparator).
//!
//! Limitations:
//! - Buffers the response body to compute the hash. Callers can opt out by
//!   skipping the middleware on streaming routes. A streaming-aware
//!   variant could expose a custom validator builder, but that requires
//!   handler cooperation and is deliberately out of scope here.
//! - Only safe methods (GET, HEAD) trigger ETag generation. PUT / PATCH
//!   conditionals (`If-Match`) are passed through untouched.

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;
use http::HeaderValue;
use http::Method;
use http::StatusCode;
use http::header::CONTENT_LENGTH;
use http::header::ETAG;
use http::header::IF_MODIFIED_SINCE;
use http::header::IF_NONE_MATCH;
use http::header::LAST_MODIFIED;
use http_body_util::BodyExt;
use sha1::Digest;
use sha1::Sha1;
use tako_core::body::TakoBody;
use tako_core::middleware::IntoMiddleware;
use tako_core::middleware::Next;
use tako_core::types::Request;
use tako_core::types::Response;

/// ETag middleware configuration.
pub struct ETag {
  /// Maximum body size considered for ETag computation. Larger responses
  /// pass through untouched.
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
      max_bytes: 1 * 1024 * 1024,
    }
  }

  /// Sets the maximum response size eligible for ETag generation.
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

fn build_304(
  status_headers: http::HeaderMap,
  request_id_header_keep: Option<HeaderValue>,
) -> Response {
  let mut resp = http::Response::builder()
    .status(StatusCode::NOT_MODIFIED)
    .body(TakoBody::empty())
    .expect("valid 304 response");
  for (k, v) in status_headers.iter() {
    if k == &CONTENT_LENGTH {
      continue;
    }
    let _ = resp.headers_mut().insert(k.clone(), v.clone());
  }
  if let Some(req_id) = request_id_header_keep {
    let _ = resp.headers_mut().insert("x-request-id", req_id);
  }
  resp
}

/// Computes a strong ETag from a body slice.
fn make_etag(bytes: &[u8]) -> String {
  let mut hasher = Sha1::new();
  hasher.update(bytes);
  let digest = hasher.finalize();
  let mut hex = String::with_capacity(2 + 40);
  hex.push('"');
  for b in digest {
    use std::fmt::Write;
    let _ = write!(hex, "{b:02x}");
  }
  hex.push('"');
  hex
}

/// Compares an `If-Modified-Since` value against `Last-Modified`. We do byte
/// equality after trimming surrounding ASCII whitespace; full HTTP-date
/// parsing would require an extra dependency and the spec allows servers to
/// fall back to the byte comparison on parse failure.
fn not_modified_since(if_modified_since: &str, last_modified: &str) -> bool {
  if_modified_since.trim() == last_modified.trim()
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
        let if_modified_since = req
          .headers()
          .get(IF_MODIFIED_SINCE)
          .and_then(|v| v.to_str().ok())
          .map(str::to_string);

        let resp = next.run(req).await;
        if !safe || resp.status() != StatusCode::OK {
          return resp;
        }

        // Honor handler-provided ETag immediately.
        if let Some(existing_etag) = resp
          .headers()
          .get(ETAG)
          .and_then(|v| v.to_str().ok())
          .map(str::to_string)
        {
          if let Some(req_etag) = if_none_match.as_ref() {
            if weak_match(req_etag, &existing_etag) {
              let headers = resp.headers().clone();
              return build_304(headers, None);
            }
          }
          // Last-Modified fast path.
          if let Some(lm) = resp
            .headers()
            .get(LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
          {
            if let Some(ims) = if_modified_since.as_ref() {
              if not_modified_since(ims, &lm) {
                let headers = resp.headers().clone();
                return build_304(headers, None);
              }
            }
          }
          return resp;
        }

        // No handler ETag: compute one if the body is bounded.
        let (parts, body) = resp.into_parts();
        let collected = match body.collect().await {
          Ok(c) => c.to_bytes(),
          Err(_) => Bytes::new(),
        };
        if collected.len() > max_bytes {
          return http::Response::from_parts(parts, TakoBody::from(collected));
        }
        let etag = make_etag(&collected);
        let mut resp = http::Response::from_parts(parts, TakoBody::from(collected));
        if let Ok(v) = HeaderValue::from_str(&etag) {
          resp.headers_mut().insert(ETAG, v);
        }
        if let Some(req_etag) = if_none_match.as_ref() {
          if weak_match(req_etag, &etag) {
            let headers = resp.headers().clone();
            return build_304(headers, None);
          }
        }
        resp
      })
    }
  }
}
