//! HMAC request-signature verification middleware (Stripe / AWS / GitHub-webhook style).
//!
//! Verifies that the request body and a configurable subset of headers were
//! signed with a shared secret using HMAC-SHA256 (RFC 2104). The signature is
//! read from a configurable header and compared in constant time.
//!
//! The default canonical string is `<method> <path>\n<body>`. Callers can
//! override the canonicalization closure for vendor-specific schemes
//! (e.g. Stripe's `t=...,v1=...` syntax or AWS Sigv4 — for the latter
//! prefer the official AWS-developed verifier).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use hmac::Hmac;
use hmac::Mac;
use http::HeaderName;
use http::StatusCode;
use http_body_util::BodyExt;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tako_core::body::TakoBody;
use tako_core::middleware::IntoMiddleware;
use tako_core::middleware::Next;
use tako_core::types::Request;
use tako_core::types::Response;

type HmacSha256 = Hmac<Sha256>;

/// Canonicalization strategy: produces the byte string that gets HMAC'd.
type CanonicalFn = Arc<dyn Fn(&http::request::Parts, &[u8]) -> Vec<u8> + Send + Sync + 'static>;

/// Signature verification middleware.
pub struct HmacSignature {
  header: HeaderName,
  secret: Vec<u8>,
  /// Maximum buffered body size. Larger requests are rejected with 413.
  max_body_bytes: usize,
  /// Canonical-string builder. Defaults to `<METHOD> <PATH>\n<BODY>`.
  canonical: CanonicalFn,
  /// When true, the literal hex digest is expected; when false, the value is
  /// expected to be base64-encoded.
  hex: bool,
  /// Optional timestamp header for replay protection. When set, both the
  /// header value is included in the default canonical string *and* its
  /// value is checked against the current wall clock with a tolerance of
  /// `max_clock_skew`. Defaults to `None` for BC; configure it via
  /// [`timestamp_header`].
  timestamp_header: Option<HeaderName>,
  /// Allowed wall-clock skew for the timestamp header. Defaults to 5 min.
  max_clock_skew: std::time::Duration,
}

impl HmacSignature {
  /// Creates the middleware. `header` is the request header carrying the
  /// signature, `secret` is the shared HMAC key.
  pub fn new(header: HeaderName, secret: impl Into<Vec<u8>>) -> Self {
    Self {
      header,
      secret: secret.into(),
      max_body_bytes: 1024 * 1024,
      canonical: Arc::new(default_canonical),
      hex: true,
      timestamp_header: None,
      max_clock_skew: std::time::Duration::from_secs(300),
    }
  }

  /// Maximum body size eligible for verification.
  pub fn max_body_bytes(mut self, n: usize) -> Self {
    self.max_body_bytes = n;
    self
  }

  /// Plug a custom canonicalization closure.
  ///
  /// **Replay-protection**: the default canonical (`METHOD\nPATH\nBODY`)
  /// gives no protection against an attacker replaying a captured request.
  /// If you keep the default, also call [`Self::timestamp_header`] so the
  /// middleware additionally binds the signature to a freshness window. A
  /// custom closure is responsible for incorporating its own freshness
  /// inputs (timestamp, nonce, etc.).
  pub fn canonical<F>(mut self, f: F) -> Self
  where
    F: Fn(&http::request::Parts, &[u8]) -> Vec<u8> + Send + Sync + 'static,
  {
    self.canonical = Arc::new(f);
    self
  }

  /// Switch between hex (default) and base64 encodings of the signature.
  pub fn hex(mut self, hex: bool) -> Self {
    self.hex = hex;
    self
  }

  /// Bind the signature to a timestamp header for replay protection.
  ///
  /// The default canonical adds the header's value as a new line between the
  /// path and the body, and the middleware rejects requests whose timestamp
  /// (Unix seconds, integer) is more than `max_clock_skew` away from `now`.
  /// Pair this with [`Self::max_clock_skew`] to tune tolerance.
  pub fn timestamp_header(mut self, header: HeaderName) -> Self {
    self.timestamp_header = Some(header);
    self
  }

  /// Tolerance for [`Self::timestamp_header`] validation. Default 5 min.
  pub fn max_clock_skew(mut self, d: std::time::Duration) -> Self {
    self.max_clock_skew = d;
    self
  }
}

fn default_canonical(parts: &http::request::Parts, body: &[u8]) -> Vec<u8> {
  let mut out =
    Vec::with_capacity(parts.method.as_str().len() + parts.uri.path().len() + 2 + body.len());
  out.extend_from_slice(parts.method.as_str().as_bytes());
  out.push(b' ');
  out.extend_from_slice(parts.uri.path().as_bytes());
  out.push(b'\n');
  // Bind the signature to the timestamp header if the request supplies one,
  // so the default canonical participates in [`timestamp_header`]-based
  // replay protection automatically. The header name to look up is
  // discovered by scanning common conventions; custom canonical closures
  // can override this for vendor-specific schemes.
  for name in ["x-timestamp", "x-signature-timestamp", "date"] {
    if let Some(v) = parts
      .headers
      .get(name)
      .and_then(|v| v.to_str().ok().map(str::trim))
    {
      out.extend_from_slice(v.as_bytes());
      out.push(b'\n');
      break;
    }
  }
  out.extend_from_slice(body);
  out
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
  if !s.len().is_multiple_of(2) {
    return None;
  }
  let bytes: Result<Vec<u8>, _> = (0..s.len())
    .step_by(2)
    .map(|i| u8::from_str_radix(&s[i..i + 2], 16))
    .collect();
  bytes.ok()
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
  use base64::Engine;
  base64::engine::general_purpose::STANDARD.decode(s).ok()
}

impl IntoMiddleware for HmacSignature {
  // The `.expect("valid response")` calls below are unreachable in practice:
  // every `http::Response::builder()` site sets only a static
  // `StatusCode::*` constant and a body produced from `TakoBody::*` — none of
  // the builder operations that can fail (custom header names with
  // non-ASCII characters, etc.) are exercised. Treating these as panics
  // rather than threading `Result` makes the early-return shape readable; if
  // the underlying `http` API ever changes such that the constraint stops
  // holding, the panic will surface immediately in tests.
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let header = self.header;
    let secret = Arc::new(self.secret);
    let canonical = self.canonical;
    let max_body_bytes = self.max_body_bytes;
    let hex = self.hex;
    let timestamp_header = self.timestamp_header;
    let max_clock_skew = self.max_clock_skew;

    move |req: Request, next: Next| {
      let header = header.clone();
      let secret = secret.clone();
      let canonical = canonical.clone();
      let timestamp_header = timestamp_header.clone();
      Box::pin(async move {
        // Replay protection: when `timestamp_header` is configured, reject
        // requests outside the allowed skew window BEFORE checking the
        // signature so the rejection cost stays cheap.
        if let Some(ts_header) = timestamp_header.as_ref() {
          let ts_str = req
            .headers()
            .get(ts_header)
            .and_then(|v| v.to_str().ok())
            .map(str::trim);
          let Some(ts) = ts_str.and_then(|s| s.parse::<i64>().ok()) else {
            return http::Response::builder()
              .status(StatusCode::UNAUTHORIZED)
              .body(TakoBody::from("missing or malformed timestamp header"))
              .expect("valid response");
          };
          let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
          // PMW-03: attacker-controlled `ts` can be `i64::MIN`; plain
          // `now - ts` then panics in debug (overflow-checks) and wraps in
          // release (signed-overflow UB-equivalent for our purposes).
          // Promote to i128 for the difference so the result is always
          // representable, then clamp the absolute skew to u64 for the
          // comparison.
          let skew_abs: u128 = (i128::from(now) - i128::from(ts)).unsigned_abs();
          if skew_abs > u128::from(max_clock_skew.as_secs()) {
            return http::Response::builder()
              .status(StatusCode::UNAUTHORIZED)
              .body(TakoBody::from("timestamp outside allowed skew"))
              .expect("valid response");
          }
        }
        let provided = req
          .headers()
          .get(&header)
          .and_then(|v| v.to_str().ok())
          .map(str::trim)
          .map(str::to_string);
        let Some(provided) = provided.filter(|s| !s.is_empty()) else {
          return http::Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(TakoBody::from("missing signature header"))
            .expect("valid response");
        };
        let provided_bytes = if hex {
          hex_decode(&provided)
        } else {
          base64_decode(&provided)
        };
        let Some(provided_bytes) = provided_bytes else {
          return http::Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(TakoBody::from("malformed signature"))
            .expect("valid response");
        };

        let (parts, body) = req.into_parts();
        let limited = http_body_util::Limited::new(body, max_body_bytes);
        let collected = match limited.collect().await {
          Ok(c) => c.to_bytes(),
          Err(_) => {
            return http::Response::builder()
              .status(StatusCode::PAYLOAD_TOO_LARGE)
              .body(TakoBody::empty())
              .expect("valid response");
          }
        };
        let canonical_bytes = (canonical)(&parts, &collected);

        let Ok(mut mac) = HmacSha256::new_from_slice(&secret) else {
          return http::Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(TakoBody::from("signer key invalid"))
            .expect("valid response");
        };
        mac.update(&canonical_bytes);
        let computed = mac.finalize().into_bytes();

        let computed_bytes: &[u8] = computed.as_ref();
        let ok = if computed_bytes.len() == provided_bytes.len() {
          bool::from(computed_bytes.ct_eq(provided_bytes.as_slice()))
        } else {
          false
        };
        if !ok {
          return http::Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(TakoBody::from("signature mismatch"))
            .expect("valid response");
        }

        // Re-inject the body for downstream handlers.
        let new_req = http::Request::from_parts(parts, TakoBody::from(collected));
        next.run(new_req).await
      })
    }
  }
}
