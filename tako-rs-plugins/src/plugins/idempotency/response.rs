//! Conflict, bad-gateway, and cache-replay response construction, plus the
//! header filter that decides which response headers survive into the cache.

use http::HeaderName;
use http::HeaderValue;
use http::StatusCode;
use http::header::CONTENT_LENGTH;
use http::header::RETRY_AFTER;
use tako_rs_core::body::TakoBody;
use tako_rs_core::types::Response;

use super::store::CachedResponse;

/// 409 response for a permanent Idempotency-Key collision — the cached
/// entry exists but the request payload differs. Clients **should not**
/// retry the same request unchanged; they must either change the key or
/// alter the payload, hence no `Retry-After`.
pub(crate) fn conflict() -> Response {
  conflict_response(None)
}

/// 409 response for a transient collision: another worker is currently
/// processing the same Idempotency-Key, or coalescing is disabled.
/// Clients **may** retry after the suggested delay (3s).
pub(crate) fn conflict_inflight() -> Response {
  conflict_response(Some(3))
}

/// PPL-18: shared builder so both 409 paths use the same response
/// shape — only the optional \`Retry-After\` differs, signalling
/// transient (`Some`) vs permanent (`None`) collisions.
fn conflict_response(retry_after_secs: Option<u32>) -> Response {
  let mut resp = http::Response::builder()
    .status(StatusCode::CONFLICT)
    .body(TakoBody::empty())
    .unwrap();
  if let Some(secs) = retry_after_secs {
    resp.headers_mut().insert(
      RETRY_AFTER,
      HeaderValue::from_str(&secs.to_string()).unwrap_or_else(|_| HeaderValue::from_static("3")),
    );
  }
  resp
}

/// Emitted when the downstream handler's response body fails to collect
/// (transient I/O error mid-stream). Returning 502 is preferable to silently
/// caching an empty body and serving it on every replay — see PPL-09.
pub(crate) fn bad_gateway() -> Response {
  http::Response::builder()
    .status(StatusCode::BAD_GATEWAY)
    .body(TakoBody::empty())
    .unwrap()
}

pub(crate) fn build_response_from_cache(c: &CachedResponse) -> Response {
  // `Response::builder().status(...).headers_mut()` returns `None` and panics
  // on `.unwrap()` whenever the builder is in an error state (the same way
  // `Response::builder().status(0u16)` would be). We never reach that path
  // because `c.status` is a real `StatusCode`, but go through a fallible
  // emit and fall back to an internal-error response so future refactors
  // that change `CachedResponse::status` to a free-form integer don't
  // re-introduce a panic on the cache replay path.
  let mut b = http::Response::builder().status(c.status);
  let Some(headers) = b.headers_mut() else {
    return http::Response::builder()
      .status(StatusCode::INTERNAL_SERVER_ERROR)
      .body(TakoBody::empty())
      .expect("static 500 builder");
  };
  for (k, v) in &c.headers {
    let _ = headers.insert(k, v.clone());
  }
  headers.remove(CONTENT_LENGTH);
  b.body(TakoBody::from(c.body.clone())).unwrap_or_else(|_| {
    http::Response::builder()
      .status(StatusCode::INTERNAL_SERVER_ERROR)
      .body(TakoBody::empty())
      .expect("static 500 builder")
  })
}

/// Pick which response headers survive into the idempotency cache.
///
/// PPL-11: previously this was an *allow-list* (only `Content-Type`,
/// `Location`, and `x-*` headers passed through). That silently dropped
/// many headers that are perfectly safe to replay — `Cache-Control`,
/// `ETag`, `Last-Modified`, `Vary`, `Link`, `Content-Language`,
/// `Content-Disposition`, `Allow`, etc. — so intermediaries lost
/// validation tokens and clients lost download filenames / language hints
/// on every replay.
///
/// Switch to a *denylist*: keep everything except headers that are unsafe
/// or wrong to replay verbatim — hop-by-hop headers (RFC 9110 §7.6.1),
/// `Content-Length` (the cached body's length may differ if size-capping
/// rewrote it), and `Set-Cookie` (replaying old cookies is a security
/// hazard — different requests should get fresh session state).
pub(crate) fn filter_headers(src: &http::HeaderMap) -> Vec<(HeaderName, HeaderValue)> {
  // Hop-by-hop headers (RFC 9110 §7.6.1) + others that must not be
  // replayed from cache.
  const DENY: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    // Content-Length: rewritten downstream after cache replay; if the
    // cached body was truncated by max_cached_body_bytes the original
    // length would lie.
    "content-length",
    // Set-Cookie: replaying old session tokens to a new caller is a
    // security risk. Sessions must be re-established each request.
    "set-cookie",
  ];
  let mut out = Vec::with_capacity(src.keys_len());
  for (name, v) in src {
    let name_lc = name.as_str().to_ascii_lowercase();
    if DENY.contains(&name_lc.as_str()) {
      continue;
    }
    out.push((name.clone(), v.clone()));
  }
  out
}
