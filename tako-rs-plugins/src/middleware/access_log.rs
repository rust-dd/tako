//! Structured access log emitted after each response, independent of the metrics pipeline.
//! `request_id` is set only when [`RequestId`](super::request_id::RequestId) runs first.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::conn_info::PeerAddr;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

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

pub struct AccessLog {
  sink: SinkFn,
}

impl Default for AccessLog {
  fn default() -> Self {
    Self::new()
  }
}

impl AccessLog {
  /// Logs through `tracing` at INFO with target `tako::access`.
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

  /// Replaces the default `tracing` sink; it runs inline on the request task.
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
          duration_us: elapsed.as_micros().min(u128::from(u64::MAX)) as u64,
          request_id,
          peer,
        };
        sink(rec);
        resp
      })
    }
  }
}
