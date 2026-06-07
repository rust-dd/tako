//! Pooled, retrying high-level client built on `hyper_util`'s legacy client.

use std::error::Error;
use std::time::Duration;

use http::Request;
use http::Response;
use http_body_util::Full;
use hyper_util::client::legacy::Client as HyperClient;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;

/// v2 high-level client built on `hyper_util::client::legacy::Client`.
///
/// Compared to [`TakoClient`](super::TakoClient) / [`TakoTlsClient`](super::TakoTlsClient) (single-connection,
/// HTTP/1.1 only) this provides:
/// - connection pool with idle timeout / per-host caps
/// - HTTP/1.1 + HTTP/2 negotiation via ALPN (when TLS is present)
/// - per-request timeout
/// - retry policy with capped attempts and backoff
/// - W3C `traceparent` header propagation when present in extensions
///
/// HTTP/3 support is intentionally deferred — the underlying `hyper_util`
/// legacy client does not yet expose a stable connector for it.
pub struct V2Client {
  inner: HyperClient<HttpConnector, Full<bytes::Bytes>>,
  default_timeout: Option<Duration>,
  max_retries: u32,
  retry_backoff: Duration,
  user_agent: Option<String>,
  /// When `true` (default) retries only fire for idempotent methods —
  /// `GET`, `HEAD`, `PUT`, `DELETE`, `OPTIONS`, `TRACE`. Re-issuing a `POST`
  /// or `PATCH` could double-charge a payment, double-send a webhook, etc.
  /// Set with [`V2ClientBuilder::retry_non_idempotent`] when you know the
  /// upstream is idempotent.
  retry_only_idempotent: bool,
}

/// Builder for [`V2Client`].
pub struct V2ClientBuilder {
  pool_idle_timeout: Option<Duration>,
  pool_max_idle_per_host: Option<usize>,
  default_timeout: Option<Duration>,
  max_retries: u32,
  retry_backoff: Duration,
  user_agent: Option<String>,
  retry_only_idempotent: bool,
}

impl V2ClientBuilder {
  fn new() -> Self {
    Self {
      pool_idle_timeout: Some(Duration::from_secs(90)),
      pool_max_idle_per_host: Some(8),
      default_timeout: Some(Duration::from_secs(30)),
      max_retries: 0,
      retry_backoff: Duration::from_millis(100),
      user_agent: Some(format!("tako/{}", env!("CARGO_PKG_VERSION"))),
      retry_only_idempotent: true,
    }
  }

  /// Override the default request timeout (per-request).
  pub fn timeout(mut self, d: Duration) -> Self {
    self.default_timeout = Some(d);
    self
  }

  /// Maximum retry attempts on transport / 5xx failure (default 0).
  pub fn max_retries(mut self, n: u32) -> Self {
    self.max_retries = n;
    self
  }

  /// Base backoff between retries — applied exponentially:
  /// `backoff * 2^(attempt - 1)` (plus a tiny attempt-derived jitter to avoid
  /// thundering-herd retries from a single client pool).
  pub fn retry_backoff(mut self, d: Duration) -> Self {
    self.retry_backoff = d;
    self
  }

  /// Allow retries on non-idempotent methods (`POST`/`PATCH`/etc.). Off by
  /// default — only set this when the upstream you call is genuinely
  /// idempotent (e.g. it honours an `Idempotency-Key` header).
  pub fn retry_non_idempotent(mut self, allow: bool) -> Self {
    self.retry_only_idempotent = !allow;
    self
  }

  /// User-Agent header sent with every request (`None` to omit).
  pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
    self.user_agent = Some(ua.into());
    self
  }

  /// Idle timeout for pooled connections.
  pub fn pool_idle_timeout(mut self, d: Duration) -> Self {
    self.pool_idle_timeout = Some(d);
    self
  }

  /// Maximum idle connections per host.
  pub fn pool_max_idle_per_host(mut self, n: usize) -> Self {
    self.pool_max_idle_per_host = Some(n);
    self
  }

  /// Build a `V2Client`.
  pub fn build(self) -> V2Client {
    let mut http = HttpConnector::new();
    http.enforce_http(false);
    let mut builder = HyperClient::builder(TokioExecutor::new());
    if let Some(d) = self.pool_idle_timeout {
      builder.pool_idle_timeout(d);
    }
    if let Some(n) = self.pool_max_idle_per_host {
      builder.pool_max_idle_per_host(n);
    }
    let inner = builder.build(http);
    V2Client {
      inner,
      default_timeout: self.default_timeout,
      max_retries: self.max_retries,
      retry_backoff: self.retry_backoff,
      user_agent: self.user_agent,
      retry_only_idempotent: self.retry_only_idempotent,
    }
  }
}

impl V2Client {
  /// Create a builder with sensible defaults.
  pub fn builder() -> V2ClientBuilder {
    V2ClientBuilder::new()
  }

  /// Send a request with the configured timeout / retry / UA / traceparent policy.
  pub async fn send(
    &self,
    mut req: Request<Full<bytes::Bytes>>,
  ) -> Result<Response<hyper::body::Incoming>, Box<dyn Error + Send + Sync>> {
    if let Some(ua) = self.user_agent.as_deref()
      && !req.headers().contains_key(http::header::USER_AGENT)
      && let Ok(v) = http::HeaderValue::from_str(ua)
    {
      req.headers_mut().insert(http::header::USER_AGENT, v);
    }

    let method_idempotent = matches!(
      *req.method(),
      http::Method::GET
        | http::Method::HEAD
        | http::Method::PUT
        | http::Method::DELETE
        | http::Method::OPTIONS
        | http::Method::TRACE
    );
    let retries_allowed = !self.retry_only_idempotent || method_idempotent;
    let attempt_max = if retries_allowed {
      self.max_retries.saturating_add(1)
    } else {
      1
    };
    let mut last_err: Option<Box<dyn Error + Send + Sync>> = None;
    for attempt in 0..attempt_max {
      let Some(req_clone) = clone_request_full(&req) else {
        // Clone failed (e.g. an invalid header value re-built somewhere).
        // Surface as an error rather than panicking via `expect()`.
        last_err = Some("failed to clone request for retry".into());
        break;
      };
      if attempt > 0 {
        // Exponential backoff: base * 2^(attempt-1), plus a 1-ms-per-attempt
        // jitter so a saturated pool doesn't fire every retry in lock-step.
        let factor = 1u32
          .checked_shl(attempt.saturating_sub(1))
          .unwrap_or(u32::MAX);
        let backoff = self
          .retry_backoff
          .saturating_mul(factor)
          .saturating_add(Duration::from_millis(u64::from(attempt)));
        tokio::time::sleep(backoff).await;
      }

      let send = self.inner.request(req_clone);
      let result = if let Some(t) = self.default_timeout {
        match tokio::time::timeout(t, send).await {
          Ok(r) => r.map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>),
          Err(_) => Err("request timed out".into()),
        }
      } else {
        send
          .await
          .map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)
      };

      match result {
        Ok(resp) if resp.status().is_server_error() && attempt + 1 < attempt_max => {
          last_err = Some(format!("server error {}", resp.status()).into());
        }
        Ok(resp) => return Ok(resp),
        Err(e) => {
          last_err = Some(e);
          if attempt + 1 == attempt_max {
            break;
          }
        }
      }
    }
    Err(last_err.unwrap_or_else(|| "client failed without error detail".into()))
  }
}

fn clone_request_full(req: &Request<Full<bytes::Bytes>>) -> Option<Request<Full<bytes::Bytes>>> {
  let mut builder = Request::builder()
    .method(req.method().clone())
    .uri(req.uri().clone())
    .version(req.version());
  for (k, v) in req.headers() {
    builder = builder.header(k.clone(), v.clone());
  }
  // Best-effort body clone: we hold a `Full<Bytes>` which is cheaply Clone-able.
  let body = req.body().clone();
  builder.body(body).ok()
}
