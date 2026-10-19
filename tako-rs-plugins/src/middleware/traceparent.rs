//! W3C Trace Context: a valid inbound `traceparent` continues with a fresh span id, otherwise a
//! new trace starts; [`TraceContext`] reaches handlers and both headers are echoed back.
//!
//! ```text
//! traceparent: <version>-<trace-id>-<parent-id>-<flags>
//! ```

use std::future::Future;
use std::pin::Pin;

use http::HeaderName;
use http::HeaderValue;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

/// Header carrying the W3C trace context.
pub const TRACEPARENT: HeaderName = HeaderName::from_static("traceparent");
/// Header carrying vendor-specific trace state.
pub const TRACESTATE: HeaderName = HeaderName::from_static("tracestate");

/// Ids are lowercase hex; `parent_id` is set only when a valid `traceparent` arrived.
#[derive(Debug, Clone)]
pub struct TraceContext {
  /// 16 bytes, shared by every span of the trace.
  pub trace_id: String,
  /// 8 bytes, fresh for this hop.
  pub span_id: String,
  pub parent_id: Option<String>,
  /// Only the `sampled` bit is specified.
  pub flags: u8,
  pub tracestate: Option<String>,
}

impl TraceContext {
  /// Renders the W3C `traceparent` wire format.
  pub fn to_header(&self) -> String {
    format!("00-{}-{}-{:02x}", self.trace_id, self.span_id, self.flags)
  }
}

pub struct Traceparent {
  emit_tracestate: bool,
}

impl Default for Traceparent {
  fn default() -> Self {
    Self::new()
  }
}

impl Traceparent {
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

/// OS-random UUID v4 bytes as lowercase hex; `bytes` is at most 16.
fn rand_hex(bytes: usize) -> String {
  let mut hex = uuid::Uuid::new_v4().simple().to_string();
  hex.truncate(bytes * 2);
  hex
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
