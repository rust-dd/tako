//! W3C Trace Context propagation middleware (`traceparent` / `tracestate`).
//!
//! Implements the wire format defined in <https://www.w3.org/TR/trace-context/>:
//!
//! ```text
//! traceparent: <version>-<trace-id>-<parent-id>-<flags>
//! ```
//!
//! On every request the middleware:
//!
//! 1. Parses an inbound `traceparent`. If well-formed, the [`TraceContext`]
//!    extension is populated with the existing trace-id and a freshly minted
//!    span-id (so this hop becomes the parent of any downstream call).
//! 2. If the inbound header is missing or malformed (per W3C: malformed →
//!    treat as absent), a new trace-id + span-id pair is generated.
//! 3. The resulting `traceparent` is written into the response so caches /
//!    proxies / clients can correlate.
//! 4. `tracestate` is propagated verbatim when present (max 32 list members
//!    per spec — strings longer than the limit are dropped silently).
//!
//! Handlers and downstream middleware can read [`TraceContext`] from request
//! extensions to forward the trace identifiers into outbound calls.

use std::future::Future;
use std::pin::Pin;

use http::HeaderName;
use http::HeaderValue;
use tako_core::middleware::IntoMiddleware;
use tako_core::middleware::Next;
use tako_core::types::Request;
use tako_core::types::Response;

/// Header carrying the W3C trace context.
pub const TRACEPARENT: HeaderName = HeaderName::from_static("traceparent");
/// Header carrying vendor-specific trace state.
pub const TRACESTATE: HeaderName = HeaderName::from_static("tracestate");

/// Decoded W3C trace context for the current request.
///
/// `trace_id` is shared by every span in a trace; `span_id` is unique to this
/// hop. `parent_id` carries the inbound `parent-id` value when the request
/// arrived with a usable `traceparent`, otherwise it is `None`.
#[derive(Debug, Clone)]
pub struct TraceContext {
  /// Lowercase 32-hex-char trace identifier (16 bytes).
  pub trace_id: String,
  /// Lowercase 16-hex-char span identifier (8 bytes) for this hop.
  pub span_id: String,
  /// Inbound parent span id if propagated from upstream.
  pub parent_id: Option<String>,
  /// 8-bit sampling / flags field (only the `sampled` bit is currently spec'd).
  pub flags: u8,
  /// Original `tracestate` value, if any.
  pub tracestate: Option<String>,
}

impl TraceContext {
  /// Renders the W3C `traceparent` wire format.
  pub fn to_header(&self) -> String {
    format!("00-{}-{}-{:02x}", self.trace_id, self.span_id, self.flags)
  }
}

/// Builder for the [`TraceContext`] middleware.
pub struct Traceparent {
  /// When true, emit `tracestate` in the response unchanged (when present).
  emit_tracestate: bool,
}

impl Default for Traceparent {
  fn default() -> Self {
    Self::new()
  }
}

impl Traceparent {
  /// Creates the middleware with sensible defaults.
  pub fn new() -> Self {
    Self {
      emit_tracestate: true,
    }
  }

  /// Disables echoing `tracestate` in responses (it is still readable from
  /// [`TraceContext::tracestate`] in handlers).
  pub fn skip_tracestate(mut self) -> Self {
    self.emit_tracestate = false;
    self
  }
}

fn rand_hex(bytes: usize) -> String {
  let mut buf = vec![0u8; bytes];
  // Use uuid v4 underlying RNG: cheap, no extra dep, OS-backed (`getrandom`).
  // Two UUIDs cover up to 32 random bytes — enough for a 16-byte trace id.
  let u1 = uuid::Uuid::new_v4().into_bytes();
  let u2 = uuid::Uuid::new_v4().into_bytes();
  let combined = [u1, u2].concat();
  buf.copy_from_slice(&combined[..bytes]);
  let mut out = String::with_capacity(bytes * 2);
  for b in buf {
    use std::fmt::Write;
    let _ = write!(out, "{b:02x}");
  }
  out
}

fn parse_traceparent(value: &str) -> Option<(String, String, u8)> {
  // `00-<32 hex>-<16 hex>-<2 hex>`
  let mut parts = value.split('-');
  let version = parts.next()?;
  if version != "00" {
    return None;
  }
  let trace_id = parts.next()?;
  let parent_id = parts.next()?;
  let flags = parts.next()?;
  if parts.next().is_some() {
    return None;
  }
  if trace_id.len() != 32 || !trace_id.chars().all(|c| c.is_ascii_hexdigit()) {
    return None;
  }
  if parent_id.len() != 16 || !parent_id.chars().all(|c| c.is_ascii_hexdigit()) {
    return None;
  }
  if flags.len() != 2 || !flags.chars().all(|c| c.is_ascii_hexdigit()) {
    return None;
  }
  // All-zero trace-id / parent-id are invalid per spec.
  if trace_id.bytes().all(|b| b == b'0') {
    return None;
  }
  if parent_id.bytes().all(|b| b == b'0') {
    return None;
  }
  let flags_u8 = u8::from_str_radix(flags, 16).ok()?;
  Some((
    trace_id.to_ascii_lowercase(),
    parent_id.to_ascii_lowercase(),
    flags_u8,
  ))
}

impl IntoMiddleware for Traceparent {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let emit_tracestate = self.emit_tracestate;

    move |mut req: Request, next: Next| {
      Box::pin(async move {
        let inbound = req
          .headers()
          .get(TRACEPARENT)
          .and_then(|v| v.to_str().ok())
          .map(str::to_string);
        let inbound_state = req
          .headers()
          .get(TRACESTATE)
          .and_then(|v| v.to_str().ok())
          .map(str::to_string);

        let parsed = inbound.as_ref().and_then(|h| parse_traceparent(h));
        let (trace_id, parent_id, flags) = match parsed {
          Some((tid, pid, fl)) => (tid, Some(pid), fl),
          None => (rand_hex(16), None, 0u8),
        };
        let span_id = rand_hex(8);

        let ctx = TraceContext {
          trace_id,
          span_id,
          parent_id,
          flags,
          tracestate: inbound_state.clone(),
        };
        let header_value = ctx.to_header();
        req.extensions_mut().insert(ctx);

        let mut resp = next.run(req).await;
        if let Ok(v) = HeaderValue::from_str(&header_value) {
          resp.headers_mut().insert(TRACEPARENT, v);
        }
        if emit_tracestate
          && let Some(state) = inbound_state
          && let Ok(v) = HeaderValue::from_str(&state)
        {
          resp.headers_mut().insert(TRACESTATE, v);
        }

        resp
      })
    }
  }
}
