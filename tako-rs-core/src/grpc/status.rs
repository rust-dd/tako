//! gRPC status codes, the trailer `GrpcStatus` payload, and error-response
//! construction shared across the unary and streaming responders.

use http::HeaderMap;
use http::StatusCode;

use crate::body::TakoBody;
use crate::types::Response;

/// gRPC status codes.
///
/// See <https://grpc.github.io/grpc/core/md_doc_statuscodes.html>
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GrpcStatusCode {
  Ok = 0,
  Cancelled = 1,
  Unknown = 2,
  InvalidArgument = 3,
  DeadlineExceeded = 4,
  NotFound = 5,
  AlreadyExists = 6,
  PermissionDenied = 7,
  ResourceExhausted = 8,
  FailedPrecondition = 9,
  Aborted = 10,
  OutOfRange = 11,
  Unimplemented = 12,
  Internal = 13,
  Unavailable = 14,
  DataLoss = 15,
  Unauthenticated = 16,
}

/// Percent-encode a gRPC `Status-Message` per PROTOCOL-HTTP2.md.
///
/// The spec preserves visible ASCII (`0x20..=0x7E`) except `%`, and
/// percent-encodes every other byte as `%XX` (upper-case hex). Without
/// this any non-ASCII character (emoji, accents, Latin-1 upstream error
/// strings) makes `HeaderValue::from_str` fail and the surrounding
/// `if let Ok(...)` silently drops the entire `grpc-message` — the
/// caller would see only `grpc-status` with no human-readable detail.
fn percent_encode_grpc_message(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  for &b in s.as_bytes() {
    if (0x20..=0x7E).contains(&b) && b != b'%' {
      out.push(b as char);
    } else {
      out.push('%');
      out.push(hex_upper(b >> 4));
      out.push(hex_upper(b & 0x0F));
    }
  }
  out
}

#[inline]
fn hex_upper(n: u8) -> char {
  match n {
    0..=9 => (b'0' + n) as char,
    10..=15 => (b'A' + n - 10) as char,
    _ => unreachable!("hex_upper called with value > 15"),
  }
}

pub(crate) fn build_grpc_error_response(status: GrpcStatusCode, message: &str) -> Response {
  let mut resp = Response::new(TakoBody::empty());
  *resp.status_mut() = StatusCode::OK; // gRPC always uses 200 OK at HTTP level
  resp.headers_mut().insert(
    http::header::CONTENT_TYPE,
    http::HeaderValue::from_static("application/grpc"),
  );
  if let Ok(val) = http::HeaderValue::from_str(&(status as u8).to_string()) {
    resp.headers_mut().insert("grpc-status", val);
  }
  if !message.is_empty()
    && let Ok(val) = http::HeaderValue::from_str(&percent_encode_grpc_message(message))
  {
    resp.headers_mut().insert("grpc-message", val);
  }
  resp
}

/// gRPC status payload (status code + optional message) used in trailers.
#[derive(Debug, Clone)]
pub struct GrpcStatus {
  pub code: GrpcStatusCode,
  pub message: Option<String>,
}

impl GrpcStatus {
  pub fn ok() -> Self {
    Self {
      code: GrpcStatusCode::Ok,
      message: None,
    }
  }

  pub fn error(code: GrpcStatusCode, message: impl Into<String>) -> Self {
    Self {
      code,
      message: Some(message.into()),
    }
  }

  pub(crate) fn write_trailers(&self) -> HeaderMap {
    let mut t = HeaderMap::new();
    if let Ok(v) = http::HeaderValue::from_str(&(self.code as u8).to_string()) {
      t.insert("grpc-status", v);
    }
    if let Some(msg) = self.message.as_deref()
      && let Ok(v) = http::HeaderValue::from_str(&percent_encode_grpc_message(msg))
    {
      t.insert("grpc-message", v);
    }
    t
  }
}
