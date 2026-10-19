//! gRPC-Web framing helpers for a custom middleware: the `-text` flavor is base64 on the
//! wire, and trailers travel as a final `0x80`-flagged frame instead of HTTP trailers.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use bytes::BytesMut;
use http::HeaderMap;

/// Matches every gRPC-Web content type, the `-text` ones included.
pub fn is_grpc_web(content_type: &str) -> bool {
  let ct = content_type.to_ascii_lowercase();
  ct.starts_with("application/grpc-web")
}

/// Matches the base64 `-text` flavor.
pub fn is_grpc_web_text(content_type: &str) -> bool {
  let ct = content_type.to_ascii_lowercase();
  ct.starts_with("application/grpc-web-text")
}

/// Base64-decodes a `-text` body; a binary body is copied through.
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

/// Encodes trailers as the final `0x80`-flagged frame; `-bin` values are base64-encoded,
/// and other values that are not ASCII are dropped.
pub fn encode_trailer_frame(trailers: &HeaderMap) -> Bytes {
  let mut payload = String::new();
  for (k, v) in trailers {
    let key_str = k.as_str();
    if key_str.ends_with("-bin") {
      payload.push_str(key_str);
      payload.push_str(": ");
      payload.push_str(&STANDARD.encode(v.as_bytes()));
      payload.push_str("\r\n");
    } else if let Ok(s) = v.to_str() {
      payload.push_str(key_str);
      payload.push_str(": ");
      payload.push_str(s);
      payload.push_str("\r\n");
    }
  }
  let payload = payload.into_bytes();
  assert!(
    u32::try_from(payload.len()).is_ok(),
    "gRPC-Web trailer frame exceeds u32::MAX bytes — length-prefix would wrap"
  );
  let mut frame = BytesMut::with_capacity(5 + payload.len());
  frame.extend_from_slice(&[0x80]);
  frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
  frame.extend_from_slice(&payload);
  frame.freeze()
}

/// Base64-encodes `body` for the `-text` flavor; a binary body passes through.
pub fn encode_response_body(content_type: &str, body: Bytes) -> Bytes {
  if is_grpc_web_text(content_type) {
    let s = STANDARD.encode(body);
    Bytes::from(s.into_bytes())
  } else {
    body
  }
}
