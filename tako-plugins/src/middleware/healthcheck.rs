//! Liveness / readiness / drain probe middleware.
//!
//! Reserves three URL paths that short-circuit the router so health probes
//! never traverse user middleware:
//!
//! - `live_path` — process is alive (always 200).
//! - `ready_path` — readiness gate. Returns 200 when the configured probes
//!   are all healthy, 503 otherwise.
//! - `drain_path` — admin endpoint that toggles the readiness gate so a load
//!   balancer can deregister this instance before shutdown. Issuing a
//!   `POST /__drain` flips the gate to "draining"; further `GET /ready` will
//!   return 503 with `Retry-After`.
//!
//! Probes are async closures that return `Result<(), String>`. On error the
//! readiness response includes the failed probe name and message.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use http::HeaderValue;
use http::Method;
use http::StatusCode;
use http::header::CONTENT_TYPE;
use http::header::RETRY_AFTER;
use tako_core::body::TakoBody;
use tako_core::middleware::IntoMiddleware;
use tako_core::middleware::Next;
use tako_core::types::Request;
use tako_core::types::Response;

type ProbeFn = Arc<
  dyn Fn() -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'static>>
    + Send
    + Sync
    + 'static,
>;

/// Single readiness probe (name + async check).
#[derive(Clone)]
pub struct Probe {
  pub name: &'static str,
  check: ProbeFn,
}

impl Probe {
  /// Wraps an async closure as a probe.
  pub fn new<F, Fut>(name: &'static str, f: F) -> Self
  where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), String>> + Send + 'static,
  {
    Self {
      name,
      check: Arc::new(move || Box::pin(f())),
    }
  }
}

/// Healthcheck middleware configuration.
pub struct Healthcheck {
  live_path: String,
  ready_path: String,
  drain_path: String,
  drain_token: Option<String>,
  retry_after_secs: u32,
  probes: Vec<Probe>,
  drained: Arc<AtomicBool>,
}

impl Default for Healthcheck {
  fn default() -> Self {
    Self::new()
  }
}

impl Healthcheck {
  /// Creates a healthcheck middleware with `/live`, `/ready`, `/__drain`.
  pub fn new() -> Self {
    Self {
      live_path: "/live".to_string(),
      ready_path: "/ready".to_string(),
      drain_path: "/__drain".to_string(),
      drain_token: None,
      retry_after_secs: 30,
      probes: Vec::new(),
      drained: Arc::new(AtomicBool::new(false)),
    }
  }

  /// Overrides the liveness path.
  pub fn live_path(mut self, p: impl Into<String>) -> Self {
    self.live_path = p.into();
    self
  }

  /// Overrides the readiness path.
  pub fn ready_path(mut self, p: impl Into<String>) -> Self {
    self.ready_path = p.into();
    self
  }

  /// Overrides the drain admin path.
  pub fn drain_path(mut self, p: impl Into<String>) -> Self {
    self.drain_path = p.into();
    self
  }

  /// Requires this token (`X-Drain-Token` header) to flip the drain gate. If
  /// set and the header doesn't match, the drain endpoint returns 401.
  pub fn drain_token(mut self, t: impl Into<String>) -> Self {
    self.drain_token = Some(t.into());
    self
  }

  /// `Retry-After` value emitted on `/ready` while the gate is closed.
  pub fn retry_after_secs(mut self, secs: u32) -> Self {
    self.retry_after_secs = secs;
    self
  }

  /// Adds a readiness probe; called sequentially on every `/ready` hit.
  pub fn probe(mut self, p: Probe) -> Self {
    self.probes.push(p);
    self
  }

  /// Returns a handle that lets the application flip the drain gate
  /// programmatically (e.g. from a `SIGTERM` handler).
  pub fn handle(&self) -> HealthcheckHandle {
    HealthcheckHandle {
      drained: self.drained.clone(),
    }
  }
}

/// Programmatic handle for flipping the drain gate from outside the request
/// pipeline.
#[derive(Clone)]
pub struct HealthcheckHandle {
  drained: Arc<AtomicBool>,
}

impl HealthcheckHandle {
  /// Marks the instance as draining (subsequent `/ready` returns 503).
  pub fn drain(&self) {
    self.drained.store(true, Ordering::Release);
  }

  /// Reverses a previous drain. Useful in tests.
  pub fn undrain(&self) {
    self.drained.store(false, Ordering::Release);
  }

  /// Reads the current drain state.
  pub fn is_draining(&self) -> bool {
    self.drained.load(Ordering::Acquire)
  }
}

fn json_response(status: StatusCode, body: String) -> Response {
  let mut resp = http::Response::builder()
    .status(status)
    .body(TakoBody::from(body))
    .expect("valid health response");
  resp
    .headers_mut()
    .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
  resp
}

impl IntoMiddleware for Healthcheck {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let live_path = Arc::new(self.live_path);
    let ready_path = Arc::new(self.ready_path);
    let drain_path = Arc::new(self.drain_path);
    let drain_token = self.drain_token.map(Arc::new);
    let retry_after = self.retry_after_secs;
    let probes = Arc::new(self.probes);
    let drained = self.drained;

    move |req: Request, next: Next| {
      let live_path = live_path.clone();
      let ready_path = ready_path.clone();
      let drain_path = drain_path.clone();
      let drain_token = drain_token.clone();
      let probes = probes.clone();
      let drained = drained.clone();

      Box::pin(async move {
        let path = req.uri().path();

        if path == live_path.as_str() && req.method() == Method::GET {
          return json_response(StatusCode::OK, r#"{"status":"alive"}"#.to_string());
        }

        if path == ready_path.as_str() && req.method() == Method::GET {
          if drained.load(Ordering::Acquire) {
            let mut resp = json_response(
              StatusCode::SERVICE_UNAVAILABLE,
              r#"{"status":"draining"}"#.to_string(),
            );
            if let Ok(v) = HeaderValue::from_str(&retry_after.to_string()) {
              resp.headers_mut().insert(RETRY_AFTER, v);
            }
            return resp;
          }

          let mut failures: Vec<(String, String)> = Vec::new();
          for probe in probes.iter() {
            if let Err(e) = (probe.check)().await {
              failures.push((probe.name.to_string(), e));
            }
          }

          if failures.is_empty() {
            return json_response(StatusCode::OK, r#"{"status":"ready"}"#.to_string());
          }

          let detail: Vec<serde_json::Value> = failures
            .into_iter()
            .map(|(n, e)| {
              serde_json::json!({
                "probe": n,
                "error": e,
              })
            })
            .collect();
          let body = serde_json::json!({
            "status": "unready",
            "failures": detail,
          });
          let mut resp = json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            serde_json::to_string(&body).unwrap_or_default(),
          );
          if let Ok(v) = HeaderValue::from_str(&retry_after.to_string()) {
            resp.headers_mut().insert(RETRY_AFTER, v);
          }
          return resp;
        }

        if path == drain_path.as_str() {
          if let Some(expected) = drain_token.as_ref() {
            let provided = req
              .headers()
              .get("x-drain-token")
              .and_then(|v| v.to_str().ok())
              .unwrap_or("");
            if provided != expected.as_str() {
              return json_response(
                StatusCode::UNAUTHORIZED,
                r#"{"error":"invalid drain token"}"#.to_string(),
              );
            }
          }
          match *req.method() {
            Method::POST => {
              drained.store(true, Ordering::Release);
              return json_response(StatusCode::OK, r#"{"status":"draining"}"#.to_string());
            }
            Method::DELETE => {
              drained.store(false, Ordering::Release);
              return json_response(StatusCode::OK, r#"{"status":"undrained"}"#.to_string());
            }
            Method::GET => {
              let body = if drained.load(Ordering::Acquire) {
                r#"{"draining":true}"#
              } else {
                r#"{"draining":false}"#
              };
              return json_response(StatusCode::OK, body.to_string());
            }
            _ => {
              return json_response(
                StatusCode::METHOD_NOT_ALLOWED,
                r#"{"error":"use GET, POST or DELETE"}"#.to_string(),
              );
            }
          }
        }

        next.run(req).await
      })
    }
  }
}
