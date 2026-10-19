//! HMAC-SHA256 request signatures (Stripe and GitHub webhook style), compared in constant
//! time. Without `timestamp_header`, a captured request stays replayable indefinitely.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use hmac::Hmac;
use hmac::KeyInit;
use hmac::Mac;
use http::HeaderName;
use http::StatusCode;
use http_body_util::BodyExt;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tako_rs_core::body::TakoBody;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

type HmacSha256 = Hmac<Sha256>;

/// Canonicalization strategy: produces the byte string that gets HMAC'd.
type CanonicalFn = Arc<dyn Fn(&http::request::Parts, &[u8]) -> Vec<u8> + Send + Sync + 'static>;

pub struct HmacSignature {
  header: HeaderName,
  secret: Vec<u8>,
  max_body_bytes: usize,
  canonical: Option<CanonicalFn>,
  hex: bool,
  timestamp_header: Option<HeaderName>,
  max_clock_skew: std::time::Duration,
}

impl HmacSignature {
  /// Expects a hex digest in `header`; bodies are capped at 1 MiB.
  pub fn new(header: HeaderName, secret: impl Into<Vec<u8>>) -> Self {
    Self {
      header,
      secret: secret.into(),
      max_body_bytes: 1024 * 1024,
      canonical: None,
      hex: true,
      timestamp_header: None,
      max_clock_skew: std::time::Duration::from_secs(300),
    }
  }

  /// Larger bodies are rejected with 413.
  pub fn max_body_bytes(mut self, n: usize) -> Self {
    self.max_body_bytes = n;
    self
  }

  /// Replaces the default `METHOD PATH\n[timestamp\n]body`; a custom closure must sign its
  /// own freshness input (timestamp, nonce) to resist replay.
  pub fn canonical<F>(mut self, f: F) -> Self
  where
    F: Fn(&http::request::Parts, &[u8]) -> Vec<u8> + Send + Sync + 'static,
  {
    self.canonical = Some(Arc::new(f));
    self
  }

  /// Switch between hex (default) and base64 encodings of the signature.
  pub fn hex(mut self, hex: bool) -> Self {
    self.hex = hex;
    self
  }

  /// Rejects requests whose Unix-seconds timestamp in `header` is further than
  /// [`Self::max_clock_skew`] from now. The default canonicalization signs this header.
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

fn default_canonical(
  parts: &http::request::Parts,
  body: &[u8],
  timestamp_header: Option<&HeaderName>,
) -> Vec<u8> {
  let mut out =
    Vec::with_capacity(parts.method.as_str().len() + parts.uri.path().len() + 2 + body.len());
  out.extend_from_slice(parts.method.as_str().as_bytes());
  out.push(b' ');
  out.extend_from_slice(parts.uri.path().as_bytes());
  out.push(b'\n');
  let timestamp = if let Some(name) = timestamp_header {
    parts
      .headers
      .get(name)
      .and_then(|v| v.to_str().ok().map(str::trim))
  } else {
    ["x-timestamp", "x-signature-timestamp", "date"]
      .into_iter()
      .find_map(|name| {
        parts
          .headers
          .get(name)
          .and_then(|v| v.to_str().ok().map(str::trim))
      })
  };
  if let Some(timestamp) = timestamp {
    out.extend_from_slice(timestamp.as_bytes());
    out.push(b'\n');
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
  // The `expect`s below cannot fire: every builder sets only a status and a body.
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
        // Checked before buffering the body so stale requests are cheap to reject.
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
          // i128, so an attacker-chosen `i64::MIN` cannot overflow the subtraction.
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
        let canonical_bytes = canonical.as_ref().map_or_else(
          || default_canonical(&parts, &collected, timestamp_header.as_ref()),
          |canonical| canonical(&parts, &collected),
        );

        let Ok(mut mac) = HmacSha256::new_from_slice(&secret) else {
          return http::Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(TakoBody::from("signer key invalid"))
            .expect("valid response");
        };
        mac.update(&canonical_bytes);
        let computed = mac.finalize().into_bytes();

        let computed_bytes: &[u8] = computed.as_ref();
        if !bool::from(computed_bytes.ct_eq(provided_bytes.as_slice())) {
          return http::Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(TakoBody::from("signature mismatch"))
            .expect("valid response");
        }

        let new_req = http::Request::from_parts(parts, TakoBody::from(collected));
        next.run(new_req).await
      })
    }
  }
}
