#![cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
//! Idempotency-Key based request de-duplication plugin.
//!
//! This plugin implements server-side idempotency for unsafe methods (typically POST),
//! keyed by a caller-provided header (default: `Idempotency-Key`). For a given key and
//! scope, it ensures that concurrent or repeated requests return the exact same response
//! (status, selected headers, body) within a configurable TTL.
//!
//! Behavior:
//! - First request with a new key is processed normally while marking the key as in-flight.
//! - Concurrent requests with the same key wait for completion and receive the cached result.
//! - Replays within TTL return the cached result immediately.
//! - If the same key is reused with a different payload, a 409 Conflict is returned.
//!
//! Notes:
//! - Bodies are buffered to compute a stable payload signature and to cache responses.
//! - Response headers are filtered to exclude hop-by-hop and length-specific headers.
//! - Storage is in-memory; TTL-based cleanup runs periodically.

use std::{
  sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
  },
  time::{Duration, Instant},
};

use anyhow::Result;
use bytes::Bytes;
use dashmap::DashMap;
use http::{
  HeaderName, HeaderValue, Method, StatusCode,
  header::{CONTENT_LENGTH, CONTENT_TYPE, LOCATION, RETRY_AFTER},
};
use http_body_util::BodyExt;
use sha1::{Digest, Sha1};
use tokio::{sync::Notify, time::timeout};

use crate::{
  body::TakoBody,
  middleware::Next,
  plugins::TakoPlugin,
  responder::Responder,
  router::Router,
  types::{Request, Response},
};

/// Which request attributes are included in the idempotency key scope.
#[derive(Clone, Copy)]
pub enum Scope {
  /// Only the header value identifies the operation.
  KeyOnly,
  /// Header value combined with HTTP method and path.
  MethodAndPath,
}

/// Cache policy and matching configuration.
#[derive(Clone)]
pub struct Config {
  /// Header that carries the idempotency key.
  pub header: HeaderName,
  /// Methods to protect. Default: [POST].
  pub methods: Vec<Method>,
  /// Time-to-live for cached results (seconds). Default: 86400 (24h).
  pub ttl_secs: u64,
  /// Include method+path in the cache key. Default: MethodAndPath.
  pub scope: Scope,
  /// If true, concurrent calls with same key wait for the first to finish. Default: true.
  pub coalesce_inflight: bool,
  /// Optional timeout for waiting on in-flight (milliseconds). Default: None (wait indefinitely).
  pub inflight_wait_timeout_ms: Option<u64>,
  /// Maximum response body size to cache (bytes). Default: 1 MiB.
  pub max_cached_body_bytes: usize,
  /// Maximum request body size to hash (bytes). Requests exceeding this are rejected with 413.
  pub max_request_body_bytes: usize,
  /// If true, enforce identical payload for the same key; otherwise only the key is checked.
  pub verify_payload: bool,
  /// If true, also cache non-success statuses. Default: true.
  pub cache_error_statuses: bool,
}

impl Default for Config {
  fn default() -> Self {
    Self {
      header: HeaderName::from_static("idempotency-key"),
      methods: vec![Method::POST],
      ttl_secs: 30,
      scope: Scope::MethodAndPath,
      coalesce_inflight: true,
      inflight_wait_timeout_ms: None,
      max_cached_body_bytes: 1 * 1024 * 1024,
      max_request_body_bytes: 1 * 1024 * 1024,
      verify_payload: true,
      cache_error_statuses: true,
    }
  }
}

/// Builder for the idempotency plugin.
pub struct IdempotencyBuilder(Config);

impl IdempotencyBuilder {
  /// Start with sensible defaults.
  pub fn new() -> Self {
    Self(Config::default())
  }
  pub fn header(mut self, h: HeaderName) -> Self {
    self.0.header = h;
    self
  }
  pub fn methods(mut self, m: &[Method]) -> Self {
    self.0.methods = m.to_vec();
    self
  }
  pub fn ttl_secs(mut self, s: u64) -> Self {
    self.0.ttl_secs = s;
    self
  }
  pub fn scope(mut self, s: Scope) -> Self {
    self.0.scope = s;
    self
  }
  pub fn coalesce_inflight(mut self, yes: bool) -> Self {
    self.0.coalesce_inflight = yes;
    self
  }
  pub fn inflight_wait_timeout_ms(mut self, ms: Option<u64>) -> Self {
    self.0.inflight_wait_timeout_ms = ms;
    self
  }
  pub fn max_cached_body_bytes(mut self, n: usize) -> Self {
    self.0.max_cached_body_bytes = n;
    self
  }
  pub fn max_request_body_bytes(mut self, n: usize) -> Self {
    self.0.max_request_body_bytes = n;
    self
  }
  pub fn verify_payload(mut self, yes: bool) -> Self {
    self.0.verify_payload = yes;
    self
  }
  pub fn cache_error_statuses(mut self, yes: bool) -> Self {
    self.0.cache_error_statuses = yes;
    self
  }
  pub fn build(self) -> IdempotencyPlugin {
    IdempotencyPlugin::new(self.0)
  }
}

#[derive(Clone)]
struct CachedResponse {
  status: StatusCode,
  headers: Vec<(HeaderName, HeaderValue)>,
  body: Bytes,
}

#[derive(Clone)]
struct Completed {
  payload_sig: [u8; 20],
  cached: Arc<CachedResponse>,
  expires_at: Instant,
}

enum Entry {
  InFlight {
    payload_sig: [u8; 20],
    notify: Arc<Notify>,
    started: Instant,
  },
  Completed(Completed),
}

#[derive(Clone)]
struct Store(Arc<DashMap<String, Entry>>);

impl Store {
  fn new() -> Self {
    Self(Arc::new(DashMap::new()))
  }

  fn get(&self, k: &str) -> Option<Entry> {
    self.0.get(k).map(|e| match &*e {
      Entry::InFlight {
        payload_sig,
        notify,
        started,
      } => Entry::InFlight {
        payload_sig: *payload_sig,
        notify: notify.clone(),
        started: *started,
      },
      Entry::Completed(c) => Entry::Completed(c.clone()),
    })
  }

  fn insert_inflight(&self, k: String, payload_sig: [u8; 20]) -> Arc<Notify> {
    let notify = Arc::new(Notify::new());
    self.0.insert(
      k,
      Entry::InFlight {
        payload_sig,
        notify: notify.clone(),
        started: Instant::now(),
      },
    );
    notify
  }

  fn complete(&self, k: String, completed: Completed) {
    self.0.insert(k, Entry::Completed(completed));
  }

  fn remove(&self, k: &str) {
    let _ = self.0.remove(k);
  }

  fn retain_expired(&self) {
    let now = Instant::now();
    self.0.retain(|_, v| match v {
      Entry::Completed(c) => c.expires_at > now,
      Entry::InFlight { .. } => true,
    });
  }
}

/// Idempotency plugin. Attach at router or route level.
#[derive(Clone)]
#[doc(alias = "idempotency")]
pub struct IdempotencyPlugin {
  cfg: Config,
  store: Store,
  janitor_started: Arc<AtomicBool>,
}

impl IdempotencyPlugin {
  pub fn builder() -> IdempotencyBuilder {
    IdempotencyBuilder::new()
  }
  pub fn new(cfg: Config) -> Self {
    Self {
      cfg,
      store: Store::new(),
      janitor_started: Arc::new(AtomicBool::new(false)),
    }
  }
}

impl TakoPlugin for IdempotencyPlugin {
  fn name(&self) -> &'static str {
    "IdempotencyPlugin"
  }

  fn setup(&self, router: &Router) -> Result<()> {
    let cfg = self.cfg.clone();
    let store = self.store.clone();

    // Register middleware
    router.middleware(move |req, next| {
      let cfg = cfg.clone();
      let store = store.clone();
      async move { handle(req, next, cfg, store).await }
    });

    // Start cleanup once
    if !self.janitor_started.swap(true, Ordering::SeqCst) {
      let store = self.store.clone();
      let ttl = self.cfg.ttl_secs;
      tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(ttl.max(60).min(3600)));
        loop {
          tick.tick().await;
          store.retain_expired();
        }
      });
    }

    Ok(())
  }
}

async fn handle(req: Request, next: Next, cfg: Config, store: Store) -> impl Responder {
  // Method guard
  if !cfg.methods.iter().any(|m| m == req.method()) {
    return next.run(req).await;
  }

  // Extract key
  let key = match req.headers().get(&cfg.header) {
    Some(v) => v.to_str().unwrap_or("").to_string(),
    None => String::new(),
  };
  if key.is_empty() {
    return next.run(req).await;
  }

  // Buffer and re-inject request body (for stable hashing)
  let (parts, body) = req.into_parts();
  let collected = match body.collect().await {
    Ok(c) => c.to_bytes(),
    Err(_) => Bytes::new(),
  };
  if collected.len() > cfg.max_request_body_bytes {
    return http::Response::builder()
      .status(StatusCode::PAYLOAD_TOO_LARGE)
      .body(TakoBody::empty())
      .unwrap();
  }
  let body_bytes = collected.clone();
  let mut hasher = Sha1::new();
  if cfg.verify_payload {
    hasher.update(parts.method.as_str().as_bytes());
    hasher.update(parts.uri.path().as_bytes());
    if let Some(ct) = parts.headers.get(CONTENT_TYPE) {
      hasher.update(ct.as_bytes());
    }
    hasher.update(&body_bytes);
  }
  let sig: [u8; 20] = if cfg.verify_payload {
    hasher.finalize().into()
  } else {
    [0u8; 20]
  };

  // Put body back
  let new_req = http::Request::from_parts(parts, TakoBody::from(Bytes::from(body_bytes)));

  // Compose cache key by scope
  let cache_key = match cfg.scope {
    Scope::KeyOnly => key,
    Scope::MethodAndPath => format!("{}|{}|{}", key, new_req.method(), new_req.uri().path()),
  };

  // Fast path: completed cache hit
  if let Some(entry) = store.get(&cache_key) {
    match entry {
      Entry::Completed(c) => {
        if cfg.verify_payload && c.payload_sig != sig {
          return conflict();
        }
        return build_response_from_cache(&c.cached);
      }
      Entry::InFlight {
        payload_sig,
        notify,
        ..
      } => {
        if !cfg.coalesce_inflight {
          return conflict_inflight();
        }
        if cfg.verify_payload && payload_sig != sig {
          return conflict();
        }
        // Wait for completion
        if let Some(ms) = cfg.inflight_wait_timeout_ms {
          let _ = timeout(Duration::from_millis(ms), notify.notified()).await;
        } else {
          notify.notified().await;
        }
        if let Some(Entry::Completed(c2)) = store.get(&cache_key) {
          if cfg.verify_payload && c2.payload_sig != sig {
            return conflict();
          }
          return build_response_from_cache(&c2.cached);
        }
        // If still not completed, treat as conflict/in-progress
        return conflict_inflight();
      }
    }
  }

  // Miss: register in-flight
  let notify = store.insert_inflight(cache_key.clone(), sig);

  // Execute handler
  let mut resp = next.run(new_req).await;

  // Collect response body
  let collected = match resp.body_mut().collect().await {
    Ok(c) => c.to_bytes(),
    Err(_) => Bytes::new(),
  };
  let body_bytes = if collected.len() > cfg.max_cached_body_bytes {
    Bytes::new()
  } else {
    collected
  };

  // Build cached value (cache selection by status)
  let status = resp.status();
  let should_cache = status.is_success() || status.is_redirection() || cfg.cache_error_statuses;

  if should_cache {
    let cached = Arc::new(CachedResponse {
      status,
      headers: filter_headers(resp.headers()),
      body: body_bytes.clone(),
    });
    let completed = Completed {
      payload_sig: sig,
      cached: cached.clone(),
      expires_at: Instant::now() + Duration::from_secs(cfg.ttl_secs),
    };
    store.complete(cache_key.clone(), completed);
    notify.notify_waiters();
    // Replace body to return to the current caller
    *resp.body_mut() = TakoBody::from(Bytes::from(cached.body.clone()));
    return resp.into_response();
  } else {
    // Not caching: clean up and return original response with collected body
    store.remove(&cache_key);
    notify.notify_waiters();
    *resp.body_mut() = TakoBody::from(Bytes::from(body_bytes));
    return resp.into_response();
  }
}

fn conflict() -> Response {
  http::Response::builder()
    .status(StatusCode::CONFLICT)
    .body(TakoBody::empty())
    .unwrap()
}

fn conflict_inflight() -> Response {
  let mut resp = http::Response::builder()
    .status(StatusCode::CONFLICT)
    .body(TakoBody::empty())
    .unwrap();
  resp
    .headers_mut()
    .insert(RETRY_AFTER, HeaderValue::from_static("3"));
  resp
}

fn build_response_from_cache(c: &CachedResponse) -> Response {
  let mut b = http::Response::builder().status(c.status);
  let headers = b.headers_mut().unwrap();
  for (k, v) in &c.headers {
    let _ = headers.insert(k, v.clone());
  }
  headers.remove(CONTENT_LENGTH);
  b.body(TakoBody::from(Bytes::from(c.body.clone()))).unwrap()
}

fn filter_headers(src: &http::HeaderMap) -> Vec<(HeaderName, HeaderValue)> {
  // Hop-by-hop headers to exclude
  const EXCLUDE: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
  ];
  let mut out = Vec::new();
  for (name, v) in src.iter() {
    let name_lc = name.as_str().to_ascii_lowercase();
    if EXCLUDE.contains(&name_lc.as_str()) {
      continue;
    }
    if name == &CONTENT_LENGTH {
      continue;
    }
    // Persist common safe headers
    if name == &CONTENT_TYPE || name == &LOCATION {
      out.push((name.clone(), v.clone()));
      continue;
    }
    // Heuristic: allow custom x- headers
    if name_lc.starts_with("x-") {
      out.push((name.clone(), v.clone()));
      continue;
    }
  }
  out
}
