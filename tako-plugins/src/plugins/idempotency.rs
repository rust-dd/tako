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

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use anyhow::Result;
use bytes::Bytes;
use http::HeaderName;
use http::HeaderValue;
use http::Method;
use http::StatusCode;
use http::header::CONTENT_LENGTH;
use http::header::CONTENT_TYPE;
use http::header::RETRY_AFTER;
use http_body_util::BodyExt;
use scc::HashMap as SccHashMap;
use sha1::Digest;
use sha1::Sha1;
use tako_core::body::TakoBody;
use tako_core::middleware::Next;
use tako_core::plugins::TakoPlugin;
use tako_core::responder::Responder;
use tako_core::router::Router;
use tako_core::types::Request;
use tako_core::types::Response;
use tokio::sync::Notify;
#[cfg(not(feature = "compio"))]
use tokio::time::timeout;

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
  /// Methods to protect. Default: `[POST]`.
  pub methods: Vec<Method>,
  /// Time-to-live for cached results (seconds). Default: 86400 (24h).
  pub ttl_secs: u64,
  /// Include method+path in the cache key. Default: `MethodAndPath`.
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
      // Matches the documented default on `Config::ttl_secs` (24h).
      ttl_secs: 86400,
      scope: Scope::MethodAndPath,
      coalesce_inflight: true,
      inflight_wait_timeout_ms: None,
      max_cached_body_bytes: 1024 * 1024,
      max_request_body_bytes: 1024 * 1024,
      verify_payload: true,
      cache_error_statuses: true,
    }
  }
}

/// Builder for the idempotency plugin.
pub struct IdempotencyBuilder(Config);

impl Default for IdempotencyBuilder {
  fn default() -> Self {
    Self::new()
  }
}

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
struct Store(Arc<SccHashMap<String, Entry>>);

/// RAII guard that ensures a registered in-flight entry is cleaned up even if
/// the handler future panics or is dropped before completion. Without this,
/// coalescing waiters parked on `notify.notified()` would never observe a
/// resolution and would hang for the lifetime of the process.
struct InflightGuard {
  store: Store,
  cache_key: String,
  notify: Arc<Notify>,
  armed: bool,
}

impl InflightGuard {
  fn new(store: Store, cache_key: String, notify: Arc<Notify>) -> Self {
    Self {
      store,
      cache_key,
      notify,
      armed: true,
    }
  }

  /// Mark the guard inactive on normal completion paths — the caller has
  /// already either persisted a Completed entry or explicitly removed the
  /// in-flight one.
  fn disarm(&mut self) {
    self.armed = false;
  }
}

impl Drop for InflightGuard {
  fn drop(&mut self) {
    if self.armed {
      self.store.remove(&self.cache_key);
      self.notify.notify_waiters();
    }
  }
}

impl Store {
  fn new() -> Self {
    Self(Arc::new(SccHashMap::new()))
  }

  fn get(&self, k: &str) -> Option<Entry> {
    self.0.get_sync(k).map(|e| match &*e {
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

  /// Atomically install a fresh `InFlight` entry for `k`, or return the
  /// entry already present.
  ///
  /// This is the only race-safe alternative to a separate `get()` followed
  /// by `insert_*()`: with two pre-existing primitives, two concurrent
  /// requests for the same key could both see `None` and both call
  /// `insert_*` — duplicating handler work, losing one of the notifiers,
  /// and (after PPL-03) silently overwriting the first writer's Completed
  /// entry. `entry_sync` collapses the check-and-install into one atomic
  /// step on the same bucket lock.
  fn install_inflight_or_get_existing(
    &self,
    k: String,
    payload_sig: [u8; 20],
  ) -> Result<Arc<Notify>, Entry> {
    use scc::hash_map::Entry as MapEntry;
    match self.0.entry_sync(k) {
      MapEntry::Vacant(v) => {
        let notify = Arc::new(Notify::new());
        v.insert_entry(Entry::InFlight {
          payload_sig,
          notify: notify.clone(),
          started: Instant::now(),
        });
        Ok(notify)
      }
      MapEntry::Occupied(o) => Err(match &*o.get() {
        Entry::Completed(c) => Entry::Completed(c.clone()),
        Entry::InFlight {
          payload_sig,
          notify,
          started,
        } => Entry::InFlight {
          payload_sig: *payload_sig,
          notify: notify.clone(),
          started: *started,
        },
      }),
    }
  }

  fn complete(&self, k: String, completed: Completed) {
    // MUST be `upsert_sync`: the key already holds the matching InFlight
    // entry (planted by `install_inflight_or_get_existing` before the
    // handler ran). `insert_sync` would no-op on collision, leaving the
    // cache filled with InFlight forever and forcing every replay through
    // the 409 conflict path — i.e. the whole idempotency store would be
    // dead.
    self.0.upsert_sync(k, Entry::Completed(completed));
  }

  fn remove(&self, k: &str) {
    let _ = self.0.remove_sync(k);
  }

  fn retain_expired(&self) {
    let now = Instant::now();
    self.0.retain_sync(|_, v| match v {
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

    // Start cleanup once.
    //
    // PPL-26: both spawn paths intentionally do not retain the JoinHandle,
    // so the janitor runs for the life of the *runtime*, not the life of
    // the plugin instance. Cloning the plugin off the router (or dropping
    // every clone) does not cancel the loop. This matches the typical
    // long-lived router lifecycle but means apps that hot-swap idempotency
    // configuration will leak one janitor task per swap. If you need
    // bounded janitor lifetime, build a wrapping plugin that holds a
    // \`tokio_util::sync::CancellationToken\` shared into the spawn and
    // fire it from your own Drop impl.
    if !self.janitor_started.swap(true, Ordering::SeqCst) {
      let store = self.store.clone();
      let ttl = self.cfg.ttl_secs;

      #[cfg(not(feature = "compio"))]
      tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(ttl.clamp(5, 3600)));
        loop {
          tick.tick().await;
          store.retain_expired();
        }
      });

      #[cfg(feature = "compio")]
      compio::runtime::spawn(async move {
        let interval = Duration::from_secs(ttl.clamp(5, 3600));
        loop {
          compio::time::sleep(interval).await;
          store.retain_expired();
        }
      })
      .detach();
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
    // PMW-/PPL-17: a header value containing non-visible-ASCII bytes
    // returns Err from `to_str()`. Previously we silently substituted
    // the empty string and fell through to the pass-through branch
    // below, so a client could bypass dedup entirely by sending a
    // single 0xC3 byte in the Idempotency-Key. Surface 400 instead.
    Some(v) => match v.to_str() {
      Ok(s) if !s.is_empty() => s.to_string(),
      Ok(_) => return next.run(req).await,
      Err(_) => {
        return (
          http::StatusCode::BAD_REQUEST,
          "Idempotency-Key must be visible ASCII",
        )
          .into_response();
      }
    },
    None => return next.run(req).await,
  };

  // Buffer and re-inject request body (for stable hashing). Wrap in
  // `Limited` so a client cannot force an unbounded allocation by lying
  // about Content-Length or by streaming chunked.
  let (parts, body) = req.into_parts();
  let limited = http_body_util::Limited::new(body, cfg.max_request_body_bytes);
  let collected = match limited.collect().await {
    Ok(c) => c.to_bytes(),
    Err(_) => {
      return http::Response::builder()
        .status(StatusCode::PAYLOAD_TOO_LARGE)
        .body(TakoBody::empty())
        .unwrap();
    }
  };
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
  let new_req = http::Request::from_parts(parts, TakoBody::from(body_bytes));

  // Compose cache key by scope
  let cache_key = match cfg.scope {
    Scope::KeyOnly => key,
    Scope::MethodAndPath => format!("{}|{}|{}", key, new_req.method(), new_req.uri().path()),
  };

  // Atomically install a fresh InFlight or pick up an existing entry.
  // The previous `store.get(...)` + `store.insert_inflight(...)` pair
  // had a TOCTOU window: two concurrent requests for the same key could
  // both see `None`, both install, and end up running the handler twice
  // — exactly what idempotency exists to prevent.
  let notify = match store.install_inflight_or_get_existing(cache_key.clone(), sig) {
    Err(Entry::Completed(c)) => {
      // Skip the sig-equality check when the cached entry was recorded
      // under `verify_payload=false` (its `payload_sig` is the placeholder
      // `[0; 20]`) — flipping the flag on at runtime would otherwise turn
      // every pre-existing cached entry into a spurious 409 for clients
      // replaying the same Idempotency-Key.
      let legacy_unverified = c.payload_sig == [0u8; 20];
      if cfg.verify_payload && !legacy_unverified && c.payload_sig != sig {
        return conflict();
      }
      return build_response_from_cache(&c.cached);
    }
    Err(Entry::InFlight {
      payload_sig,
      notify,
      ..
    }) => {
      if !cfg.coalesce_inflight {
        return conflict_inflight();
      }
      let legacy_unverified = payload_sig == [0u8; 20];
      if cfg.verify_payload && !legacy_unverified && payload_sig != sig {
        return conflict();
      }
      // Wait for completion, honoring the optional timeout on both runtimes.
      if let Some(ms) = cfg.inflight_wait_timeout_ms {
        #[cfg(not(feature = "compio"))]
        {
          let _ = timeout(Duration::from_millis(ms), notify.notified()).await;
        }
        // compio's timer futures are !Send, so we cannot await them directly inside
        // a middleware handler (whose returned future is required to be Send).
        // Forward the timeout through a helper compio task that fires `Notify`
        // — `Notified` is Send, which keeps the middleware future Send-clean.
        //
        // PPL-19: hold the JoinHandle (don't `.detach()`) so dropping it
        // after the select races cancels the timer task. Otherwise the
        // sleep keeps running for the full `ms` even if the inflight
        // notify fired first, lingering as a no-op task and a delayed
        // notify_waiters on a Notify nobody is listening to.
        #[cfg(feature = "compio")]
        {
          let timeout_signal = Arc::new(Notify::new());
          let timer_signal = timeout_signal.clone();
          let _timer_task = compio::runtime::spawn(async move {
            compio::time::sleep(Duration::from_millis(ms)).await;
            timer_signal.notify_waiters();
          });
          futures_util::future::select(
            std::pin::pin!(notify.notified()),
            std::pin::pin!(timeout_signal.notified()),
          )
          .await;
          drop(_timer_task);
        }
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
    Ok(notify) => notify,
  };
  let mut inflight_guard = InflightGuard::new(store.clone(), cache_key.clone(), notify.clone());

  // Execute handler
  let mut resp = next.run(new_req).await;

  // Collect response body.
  //
  // PPL-09: a `collect().await` error means the downstream handler's body
  // stream errored mid-flight (broken upstream connection, encoding fault,
  // etc.). Previously we silently substituted `Bytes::new()` and persisted
  // a Completed entry with an empty body — sticky cache-poisoning that
  // returned silent empty 2xx (or whatever status the handler set before
  // the error) to every replay for `ttl_secs`. Instead: do NOT cache,
  // drop the inflight entry via `InflightGuard::Drop` (no `disarm` call),
  // and return 502 so the current caller sees a real failure. Coalesced
  // waiters are woken by the guard's drop and observe the absent entry
  // → `conflict_inflight()` to them, which the client retries.
  let collected = match resp.body_mut().collect().await {
    Ok(c) => c.to_bytes(),
    Err(_) => {
      // `inflight_guard` is still armed → its Drop removes the entry and
      // calls notify_waiters; no need to do it manually.
      return bad_gateway();
    }
  };
  let body_bytes = if collected.len() > cfg.max_cached_body_bytes {
    Bytes::new()
  } else {
    collected
  };

  // Build cached value (cache selection by status). Errors are cached too —
  // either for the full TTL when `cache_error_statuses=true`, or briefly
  // (1s) when `cache_error_statuses=false` so any coalesced waiter sees the
  // same response instead of getting a confusing 409 replay. Fresh requests
  // after the brief TTL bypass the cache as the flag intends.
  let status = resp.status();
  let is_error = status.is_client_error() || status.is_server_error();
  let cached = Arc::new(CachedResponse {
    status,
    headers: filter_headers(resp.headers()),
    body: body_bytes.clone(),
  });
  let ttl = if is_error && !cfg.cache_error_statuses {
    Duration::from_secs(1)
  } else {
    Duration::from_secs(cfg.ttl_secs)
  };
  let completed = Completed {
    payload_sig: sig,
    cached: cached.clone(),
    expires_at: Instant::now() + ttl,
  };
  store.complete(cache_key.clone(), completed);
  notify.notify_waiters();
  inflight_guard.disarm();
  // Replace body to return to the current caller
  *resp.body_mut() = TakoBody::from(cached.body.clone());
  resp.into_response()
}

/// 409 response for a permanent Idempotency-Key collision — the cached
/// entry exists but the request payload differs. Clients **should not**
/// retry the same request unchanged; they must either change the key or
/// alter the payload, hence no `Retry-After`.
fn conflict() -> Response {
  conflict_response(None)
}

/// 409 response for a transient collision: another worker is currently
/// processing the same Idempotency-Key, or coalescing is disabled.
/// Clients **may** retry after the suggested delay (3s).
fn conflict_inflight() -> Response {
  conflict_response(Some(3))
}

/// PPL-18: shared builder so both 409 paths use the same response
/// shape — only the optional \`Retry-After\` differs, signalling
/// transient (`Some`) vs permanent (`None`) collisions.
fn conflict_response(retry_after_secs: Option<u32>) -> Response {
  let mut resp = http::Response::builder()
    .status(StatusCode::CONFLICT)
    .body(TakoBody::empty())
    .unwrap();
  if let Some(secs) = retry_after_secs {
    resp.headers_mut().insert(
      RETRY_AFTER,
      HeaderValue::from_str(&secs.to_string())
        .unwrap_or_else(|_| HeaderValue::from_static("3")),
    );
  }
  resp
}

/// Emitted when the downstream handler's response body fails to collect
/// (transient I/O error mid-stream). Returning 502 is preferable to silently
/// caching an empty body and serving it on every replay — see PPL-09.
fn bad_gateway() -> Response {
  http::Response::builder()
    .status(StatusCode::BAD_GATEWAY)
    .body(TakoBody::empty())
    .unwrap()
}

fn build_response_from_cache(c: &CachedResponse) -> Response {
  // `Response::builder().status(...).headers_mut()` returns `None` and panics
  // on `.unwrap()` whenever the builder is in an error state (the same way
  // `Response::builder().status(0u16)` would be). We never reach that path
  // because `c.status` is a real `StatusCode`, but go through a fallible
  // emit and fall back to an internal-error response so future refactors
  // that change `CachedResponse::status` to a free-form integer don't
  // re-introduce a panic on the cache replay path.
  let mut b = http::Response::builder().status(c.status);
  let Some(headers) = b.headers_mut() else {
    return http::Response::builder()
      .status(StatusCode::INTERNAL_SERVER_ERROR)
      .body(TakoBody::empty())
      .expect("static 500 builder");
  };
  for (k, v) in &c.headers {
    let _ = headers.insert(k, v.clone());
  }
  headers.remove(CONTENT_LENGTH);
  b.body(TakoBody::from(c.body.clone())).unwrap_or_else(|_| {
    http::Response::builder()
      .status(StatusCode::INTERNAL_SERVER_ERROR)
      .body(TakoBody::empty())
      .expect("static 500 builder")
  })
}

/// Pick which response headers survive into the idempotency cache.
///
/// PPL-11: previously this was an *allow-list* (only `Content-Type`,
/// `Location`, and `x-*` headers passed through). That silently dropped
/// many headers that are perfectly safe to replay — `Cache-Control`,
/// `ETag`, `Last-Modified`, `Vary`, `Link`, `Content-Language`,
/// `Content-Disposition`, `Allow`, etc. — so intermediaries lost
/// validation tokens and clients lost download filenames / language hints
/// on every replay.
///
/// Switch to a *denylist*: keep everything except headers that are unsafe
/// or wrong to replay verbatim — hop-by-hop headers (RFC 9110 §7.6.1),
/// `Content-Length` (the cached body's length may differ if size-capping
/// rewrote it), and `Set-Cookie` (replaying old cookies is a security
/// hazard — different requests should get fresh session state).
fn filter_headers(src: &http::HeaderMap) -> Vec<(HeaderName, HeaderValue)> {
  // Hop-by-hop headers (RFC 9110 §7.6.1) + others that must not be
  // replayed from cache.
  const DENY: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    // Content-Length: rewritten downstream after cache replay; if the
    // cached body was truncated by max_cached_body_bytes the original
    // length would lie.
    "content-length",
    // Set-Cookie: replaying old session tokens to a new caller is a
    // security risk. Sessions must be re-established each request.
    "set-cookie",
  ];
  let mut out = Vec::with_capacity(src.keys_len());
  for (name, v) in src {
    let name_lc = name.as_str().to_ascii_lowercase();
    if DENY.contains(&name_lc.as_str()) {
      continue;
    }
    out.push((name.clone(), v.clone()));
  }
  out
}
