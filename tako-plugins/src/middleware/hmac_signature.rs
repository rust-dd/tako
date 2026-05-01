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

use bytes::Bytes;
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
}

impl HmacSignature {
  /// Creates the middleware. `header` is the request header carrying the
  /// signature, `secret` is the shared HMAC key.
  pub fn new(header: HeaderName, secret: impl Into<Vec<u8>>) -> Self {
    Self {
      header,
      secret: secret.into(),
      max_body_bytes: 1 * 1024 * 1024,
      canonical: Arc::new(default_canonical),
      hex: true,
    }
  }

  /// Maximum body size eligible for verification.
  pub fn max_body_bytes(mut self, n: usize) -> Self {
    self.max_body_bytes = n;
    self
  }

  /// Plug a custom canonicalization closure.
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
}

fn default_canonical(parts: &http::request::Parts, body: &[u8]) -> Vec<u8> {
  let mut out =
    Vec::with_capacity(parts.method.as_str().len() + parts.uri.path().len() + 1 + body.len());
  out.extend_from_slice(parts.method.as_str().as_bytes());
  out.push(b' ');
  out.extend_from_slice(parts.uri.path().as_bytes());
  out.push(b'\n');
  out.extend_from_slice(body);
  out
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
  if s.len() % 2 != 0 {
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

    move |req: Request, next: Next| {
      let header = header.clone();
      let secret = secret.clone();
      let canonical = canonical.clone();
      Box::pin(async move {
        let provided = req
          .headers()
          .get(&header)
          .and_then(|v| v.to_str().ok())
          .map(str::trim)
          .map(str::to_string);
        let provided = match provided {
          Some(s) if !s.is_empty() => s,
          _ => {
            return http::Response::builder()
              .status(StatusCode::UNAUTHORIZED)
              .body(TakoBody::from("missing signature header"))
              .expect("valid response");
          }
        };
        let provided_bytes = if hex {
          hex_decode(&provided)
        } else {
          base64_decode(&provided)
        };
        let provided_bytes = match provided_bytes {
          Some(b) => b,
          None => {
            return http::Response::builder()
              .status(StatusCode::BAD_REQUEST)
              .body(TakoBody::from("malformed signature"))
              .expect("valid response");
          }
        };

        let (parts, body) = req.into_parts();
        let collected = match body.collect().await {
          Ok(c) => c.to_bytes(),
          Err(_) => Bytes::new(),
        };
        if collected.len() > max_body_bytes {
          return http::Response::builder()
            .status(StatusCode::PAYLOAD_TOO_LARGE)
            .body(TakoBody::empty())
            .expect("valid response");
        }
        let canonical_bytes = (canonical)(&parts, &collected);

        let mut mac = match HmacSha256::new_from_slice(&secret) {
          Ok(m) => m,
          Err(_) => {
            return http::Response::builder()
              .status(StatusCode::INTERNAL_SERVER_ERROR)
              .body(TakoBody::from("signer key invalid"))
              .expect("valid response");
          }
        };
        mac.update(&canonical_bytes);
        let computed = mac.finalize().into_bytes();

        let computed_bytes: &[u8] = computed.as_ref();
        let ok = if computed_bytes.len() != provided_bytes.len() {
          false
        } else {
          bool::from(computed_bytes.ct_eq(provided_bytes.as_slice()))
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
