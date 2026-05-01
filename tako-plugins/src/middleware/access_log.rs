//! Structured access log middleware.
//!
//! Emits one log line per request after the response is produced, separate
//! from the metrics signal pipeline so operators can keep request-level audit
//! trails even when metrics are disabled.
//!
//! Default sink writes through the `tracing` macros at INFO level
//! (`target = "tako::access"`). Plug a custom sink via [`AccessLog::sink`] for
//! JSON / OTLP / file rotation.
//!
//! Fields per record:
//!
//! - `method`, `path`, `version`
//! - `status` (numeric)
//! - `duration_us` (microseconds)
//! - `request_id` if a [`RequestIdValue`](super::request_id::RequestIdValue)
//!   extension is present
//! - `peer` (ip / unix / other) if a [`ConnInfo`](tako_core::conn_info::ConnInfo)
//!   extension is present

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use tako_core::conn_info::ConnInfo;
use tako_core::conn_info::PeerAddr;
use tako_core::middleware::IntoMiddleware;
use tako_core::middleware::Next;
use tako_core::types::Request;
use tako_core::types::Response;

use super::request_id::RequestIdValue;

/// Single access-log record handed to the sink.
#[derive(Debug, Clone)]
pub struct AccessRecord {
  pub method: String,
  pub path: String,
  pub version: String,
  pub status: u16,
  pub duration_us: u64,
  pub request_id: Option<String>,
  pub peer: Option<String>,
}

type SinkFn = Arc<dyn Fn(AccessRecord) + Send + Sync + 'static>;

/// Access log middleware.
pub struct AccessLog {
  sink: SinkFn,
}

impl Default for AccessLog {
  fn default() -> Self {
    Self::new()
  }
}

impl AccessLog {
  /// Creates an access log middleware that writes through `tracing` at INFO.
  pub fn new() -> Self {
    Self {
      sink: Arc::new(|rec: AccessRecord| {
        tracing::info!(
          target: "tako::access",
          method = %rec.method,
          path = %rec.path,
          version = %rec.version,
          status = rec.status,
          duration_us = rec.duration_us,
          request_id = rec.request_id.as_deref(),
          peer = rec.peer.as_deref(),
          "access",
        );
      }),
    }
  }

  /// Replaces the default `tracing` sink with a custom one (JSON exporter,
  /// async channel, file rotation, …).
  pub fn sink<F>(mut self, f: F) -> Self
  where
    F: Fn(AccessRecord) + Send + Sync + 'static,
  {
    self.sink = Arc::new(f);
    self
  }
}

fn peer_label(info: &ConnInfo) -> String {
  match &info.peer {
    PeerAddr::Ip(sa) => sa.to_string(),
    PeerAddr::Unix(Some(p)) => format!("unix:{}", p.display()),
    PeerAddr::Unix(None) => "unix:?".to_string(),
    PeerAddr::Other(s) => s.clone(),
  }
}

impl IntoMiddleware for AccessLog {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let sink = self.sink;

    move |req: Request, next: Next| {
      let sink = sink.clone();
      Box::pin(async move {
        let started = Instant::now();
        let method = req.method().to_string();
        let path = req.uri().path().to_string();
        let version = format!("{:?}", req.version());
        let request_id = req
          .extensions()
          .get::<RequestIdValue>()
          .map(|v| v.0.clone());
        let peer = req.extensions().get::<ConnInfo>().map(peer_label);

        let resp = next.run(req).await;
        let elapsed = started.elapsed();
        let rec = AccessRecord {
          method,
          path,
          version,
          status: resp.status().as_u16(),
          duration_us: elapsed.as_micros().min(u64::MAX as u128) as u64,
          request_id,
          peer,
        };
        sink(rec);
        resp
      })
    }
  }
}
