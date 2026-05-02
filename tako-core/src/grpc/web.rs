//! gRPC-Web bridge — converts `application/grpc-web+proto` /
//! `application/grpc-web-text` framing to canonical `application/grpc` and
//! back, so browsers reaching a Tako gRPC route can talk to it directly.
//!
//! ⚠️ **Status:** the framing translation is provided as a free function so
//! it can be used inside a custom middleware. A full middleware adapter
//! (request-side decoder + response-side encoder + trailer-as-header
//! envelope) lives in the dev branch as a follow-up to keep this module
//! focused on the byte-level shape.
//!
//! gRPC-Web framing differs from canonical gRPC in two places:
//!
//! 1. **Wire content-type** — `application/grpc-web+proto` (binary) or
//!    `application/grpc-web-text` (base64-of-binary).
//! 2. **Trailers** — sent inline as a final `0x80`-flagged frame whose body
//!    is `key: value\r\n` lines, instead of HTTP/2 trailers.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use bytes::BytesMut;
use http::HeaderMap;

/// Identify gRPC-Web requests (binary or text variant).
pub fn is_grpc_web(content_type: &str) -> bool {
  let ct = content_type.to_ascii_lowercase();
  ct.starts_with("application/grpc-web")
}

/// Detects the text (base64) flavor of gRPC-Web.
pub fn is_grpc_web_text(content_type: &str) -> bool {
  let ct = content_type.to_ascii_lowercase();
  ct.starts_with("application/grpc-web-text")
}

/// Decode a gRPC-Web message body to canonical gRPC framing.
///
/// For `*-text` messages this base64-decodes the body first; for binary
/// messages the buffer is passed through unchanged.
pub fn decode_request_body(content_type: &str, body: &[u8]) -> Result<Bytes, String> {
  if is_grpc_web_text(content_type) {
    let decoded = STANDARD
      .decode(body)
      .map_err(|e| format!("invalid base64: {e}"))?;
    Ok(Bytes::from(decoded))
  } else {
    Ok(Bytes::copy_from_slice(body))
  }
}

/// Encode a trailer header map as the gRPC-Web `0x80`-flagged trailer frame.
///
/// Format: `[flag: u8][len: u32 BE][headers...]`, where `headers` is
/// `key: value\r\n`-encoded ASCII.
pub fn encode_trailer_frame(trailers: &HeaderMap) -> Bytes {
  let mut payload = String::new();
  for (k, v) in trailers {
    if let Ok(s) = v.to_str() {
      payload.push_str(k.as_str());
      payload.push_str(": ");
      payload.push_str(s);
      payload.push_str("\r\n");
    }
  }
  let payload = payload.into_bytes();
  let mut frame = BytesMut::with_capacity(5 + payload.len());
  frame.extend_from_slice(&[0x80]);
  frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
  frame.extend_from_slice(&payload);
  frame.freeze()
}

/// Wrap a binary gRPC-Web body for the `*-text` flavor (base64 over the wire).
pub fn encode_response_body(content_type: &str, body: Bytes) -> Bytes {
  if is_grpc_web_text(content_type) {
    let s = STANDARD.encode(body);
    Bytes::from(s.into_bytes())
  } else {
    body
  }
}
