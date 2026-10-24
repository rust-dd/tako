use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use http::StatusCode;
use http::header;
use http_body_util::BodyExt;
use sha2::Digest;
use sha2::Sha256;
use tako_rs_core::body::TakoBody;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::plugins::TakoPlugin;
use tako_rs_core::responder::Responder;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

use super::config::Config;
use super::config::IdempotencyBuilder;
use super::identity::IdentitySource;
use super::response::cacheable_body;
use super::response::conflict;
use super::response::filter_headers;
use super::response::replay;
use super::store::InflightGuard;
use crate::stores::IdempotencyBegin;
use crate::stores::IdempotencyEntry;
use crate::stores::IdempotencyStore;
use crate::stores::StoreResult;
use crate::stores::memory::MemoryIdempotencyStore;
use crate::stores::runtime;

/// Identity-scoped retries; run authentication and authorization before this layer.
///
/// Use `IntoMiddleware` for explicit per-route ordering. Unknown-length, oversized and
/// trailer-bearing responses pass through uncached. Keep lease TTL above the handler duration.
#[derive(Clone)]
#[doc(alias = "idempotency")]
pub struct IdempotencyPlugin {
  cfg: Config,
  pub(crate) identity_source: IdentitySource,
  pub(crate) store: Arc<dyn IdempotencyStore>,
  janitor_started: Arc<AtomicBool>,
}

impl IdempotencyPlugin {
  pub fn builder() -> IdempotencyBuilder {
    IdempotencyBuilder::new()
  }
  pub fn new(cfg: Config) -> Self {
    Self {
      cfg,
      identity_source: IdentitySource::default(),
      store: Arc::new(MemoryIdempotencyStore::new()),
      janitor_started: Arc::new(AtomicBool::new(false)),
    }
  }
}

impl TakoPlugin for IdempotencyPlugin {
  fn name(&self) -> &'static str {
    "IdempotencyPlugin"
  }
  fn setup(&self, router: &Router) -> Result<()> {
    router.middleware(self.clone().into_middleware());
    Ok(())
  }
}

impl IntoMiddleware for IdempotencyPlugin {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let cfg = Arc::new(self.cfg);
    let store = self.store;
    let identity_source = self.identity_source;
    let janitor_started = self.janitor_started;
    move |request, next| {
      let cfg = cfg.clone();
      let store = store.clone();
      let identity_source = identity_source.clone();
      let janitor_started = janitor_started.clone();
      Box::pin(async move {
        if !janitor_started.swap(true, Ordering::Relaxed) {
          runtime::sweep(
            &store,
            Duration::from_secs(cfg.ttl_secs.clamp(5, 3600)),
            |store| Box::pin(async move { store.sweep().await }),
          );
        }
        handle(request, next, &cfg, store, &identity_source)
          .await
          .unwrap_or_else(runtime::unavailable)
      })
    }
  }
}

async fn handle(
  request: Request,
  next: Next,
  cfg: &Config,
  store: Arc<dyn IdempotencyStore>,
  identity_source: &IdentitySource,
) -> StoreResult<Response> {
  if !cfg.methods.contains(request.method()) {
    return Ok(next.run(request).await);
  }
  let key = match request.headers().get(&cfg.header) {
    None => return Ok(next.run(request).await),
    Some(value) => match value.to_str() {
      Ok("") => return Ok(next.run(request).await),
      Ok(key) => key.to_owned(),
      Err(_) => return Ok(StatusCode::BAD_REQUEST.into_response()),
    },
  };
  let Some(cache_key) = identity_source.cache_key(&request, cfg.scope, &key) else {
    return Ok(StatusCode::UNAUTHORIZED.into_response());
  };
  let (parts, body) = request.into_parts();
  let body = match http_body_util::Limited::new(body, cfg.max_request_body_bytes)
    .collect()
    .await
  {
    Ok(body) => body.to_bytes(),
    Err(error) => {
      let status = if error.is::<http_body_util::LengthLimitError>() {
        StatusCode::PAYLOAD_TOO_LARGE
      } else {
        StatusCode::BAD_REQUEST
      };
      return Ok(status.into_response());
    }
  };
  let payload_sig = if cfg.verify_payload {
    let mut hash = Sha256::new();
    let target = parts
      .uri
      .path_and_query()
      .map_or(parts.uri.path(), http::uri::PathAndQuery::as_str);
    for bytes in [
      parts.method.as_str().as_bytes(),
      target.as_bytes(),
      parts
        .headers
        .get(header::CONTENT_TYPE)
        .map_or(&[][..], http::HeaderValue::as_bytes),
      &body,
    ] {
      hash.update((bytes.len() as u64).to_be_bytes());
      hash.update(bytes);
    }
    hash.finalize().into()
  } else {
    [0; 32]
  };
  let lease = match store.begin(&cache_key, payload_sig).await? {
    IdempotencyBegin::Acquired(lease) => lease,
    IdempotencyBegin::Existing(entry) => {
      if cfg.verify_payload && entry.payload_sig != payload_sig {
        return Ok(conflict(false));
      }
      if entry.completed {
        return replay(entry);
      }
      if !cfg.coalesce_inflight {
        return Ok(conflict(true));
      }
      return match store
        .wait(
          &cache_key,
          cfg.inflight_wait_timeout_ms.map(Duration::from_millis),
        )
        .await?
      {
        Some(entry) if cfg.verify_payload && entry.payload_sig != payload_sig => {
          Ok(conflict(false))
        }
        Some(entry) if entry.completed => replay(entry),
        _ => Ok(conflict(true)),
      };
    }
  };
  let mut guard = InflightGuard {
    store: store.clone(),
    key: cache_key,
    lease,
    armed: true,
  };
  let request = http::Request::from_parts(parts, TakoBody::from(body));
  let mut response = next.run(request).await;
  let body = match cacheable_body(&mut response, cfg.max_cached_body_bytes).await {
    Ok(Some(bytes)) => bytes,
    Ok(None) => {
      store.remove(&guard.key, &guard.lease).await?;
      guard.armed = false;
      return Ok(response);
    }
    Err(error) => {
      tracing::debug!(%error, "idempotent response body failed");
      return Ok(StatusCode::BAD_GATEWAY.into_response());
    }
  };
  let failed = response.status().is_client_error() || response.status().is_server_error();
  let ttl = Duration::from_secs(if failed && !cfg.cache_error_statuses {
    1
  } else {
    cfg.ttl_secs
  });
  let entry = IdempotencyEntry {
    status: response.status().as_u16(),
    headers: filter_headers(response.headers()),
    body,
    payload_sig,
    completed: true,
  };
  if !store.complete(&guard.key, &guard.lease, entry, ttl).await? {
    tracing::warn!("idempotency lease expired before response completion");
  }
  guard.armed = false;
  Ok(response)
}
