//! The idempotency plugin itself: builder entry point, janitor wiring, and
//! the middleware that performs key extraction, coalescing, and replay.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use anyhow::Result;
use bytes::Bytes;
use http::StatusCode;
use http::header::CONTENT_TYPE;
use http_body_util::BodyExt;
use sha1::Digest;
use sha1::Sha1;
use tako_rs_core::body::TakoBody;
use tako_rs_core::middleware::Next;
use tako_rs_core::plugins::TakoPlugin;
use tako_rs_core::responder::Responder;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;
#[cfg(feature = "compio")]
use tokio::sync::Notify;
#[cfg(not(feature = "compio"))]
use tokio::time::timeout;

use super::config::Config;
use super::config::IdempotencyBuilder;
use super::config::Scope;
use super::response::bad_gateway;
use super::response::build_response_from_cache;
use super::response::conflict;
use super::response::conflict_inflight;
use super::response::filter_headers;
use super::store::CachedResponse;
use super::store::Completed;
use super::store::Entry;
use super::store::InflightGuard;
use super::store::Store;

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
          let timer_task = compio::runtime::spawn(async move {
            compio::time::sleep(Duration::from_millis(ms)).await;
            timer_signal.notify_waiters();
          });
          futures_util::future::select(
            std::pin::pin!(notify.notified()),
            std::pin::pin!(timeout_signal.notified()),
          )
          .await;
          drop(timer_task);
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
