use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use http::HeaderValue;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

use super::cookie::SameSite;
use super::cookie::build_cookie;
use super::cookie::build_expired_cookie;
use super::cookie::extract_cookie_value;
use super::cookie::generate_session_id;
use super::data::Session;
use super::store::SessionRecord;
use super::store::SessionStoreHandle;
use super::store::SessionTtl;
use crate::stores::SessionStore;
use crate::stores::StoreResult;
use crate::stores::memory::MemorySessionStore;
use crate::stores::runtime;

struct Config {
  cookie_name: String,
  ttl: SessionTtl,
  path: String,
  domain: Option<String>,
  secure: bool,
  http_only: bool,
  same_site: SameSite,
}

/// Session middleware using memory by default or a supplied asynchronous backend.
///
/// Shared backends use Unix timestamps for the absolute lifetime; replica clocks
/// should be synchronized. Backend failures return 503 without exposing their details.
pub struct SessionMiddleware<S = MemorySessionStore> {
  config: Config,
  store: Arc<S>,
}

impl Default for SessionMiddleware {
  fn default() -> Self {
    Self::new()
  }
}
impl SessionMiddleware {
  /// Configure secure cookies, a one-hour idle TTL, and a 24-hour absolute cap.
  pub fn new() -> Self {
    Self {
      config: Config {
        cookie_name: "tako_session".into(),
        ttl: SessionTtl::default(),
        path: "/".into(),
        domain: None,
        secure: true,
        http_only: true,
        same_site: SameSite::Lax,
      },
      store: Arc::new(MemorySessionStore::new()),
    }
  }
  /// Administrative revocation for the memory backend.
  pub fn handle(&self) -> SessionStoreHandle {
    SessionStoreHandle {
      store: self.store.clone(),
    }
  }
}
impl<S: SessionStore> SessionMiddleware<S> {
  /// Replace the backend. Its clones must refer to the same shared state.
  pub fn store<T: SessionStore>(self, store: T) -> SessionMiddleware<T> {
    SessionMiddleware {
      config: self.config,
      store: Arc::new(store),
    }
  }
  /// Cookie name, default `tako_session`.
  pub fn cookie_name(mut self, name: &str) -> Self {
    self.config.cookie_name = name.into();
    self
  }
  /// Set idle lifetime without changing the absolute cap.
  pub fn ttl_secs(mut self, seconds: u64) -> Self {
    self.config.ttl.idle_secs = seconds;
    self
  }
  /// Set the full expiration policy.
  pub fn ttl(mut self, ttl: SessionTtl) -> Self {
    self.config.ttl = ttl;
    self
  }
  /// Cookie path, default `/`.
  pub fn path(mut self, path: &str) -> Self {
    self.config.path = path.into();
    self
  }
  /// Optional cookie domain.
  pub fn domain(mut self, domain: &str) -> Self {
    self.config.domain = Some(domain.into());
    self
  }
  /// Cookie `Secure` flag, default true.
  pub fn secure(mut self, secure: bool) -> Self {
    self.config.secure = secure;
    self
  }
  /// Cookie `HttpOnly` flag, default true.
  pub fn http_only(mut self, enabled: bool) -> Self {
    self.config.http_only = enabled;
    self
  }
  /// Cookie `SameSite` policy, default Lax.
  pub fn same_site(mut self, value: SameSite) -> Self {
    self.config.same_site = value;
    self
  }
}

impl<S: SessionStore> IntoMiddleware for SessionMiddleware<S> {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let config = Arc::new(self.config);
    let store = self.store;
    let started = Arc::new(AtomicBool::new(false));
    move |request: Request, next: Next| {
      if !started.swap(true, Ordering::Relaxed) {
        runtime::sweep(
          &store,
          Duration::from_secs(config.ttl.idle_secs.clamp(60, 3600)),
          |store| Box::pin(async move { store.sweep().await }),
        );
      }
      let config = config.clone();
      let store = store.clone();
      Box::pin(async move {
        handle(request, next, &config, store.as_ref())
          .await
          .unwrap_or_else(runtime::unavailable)
      })
    }
  }
}

async fn handle<S: SessionStore>(
  mut request: Request,
  next: Next,
  config: &Config,
  store: &S,
) -> StoreResult<Response> {
  let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
  let inbound = extract_cookie_value(&request, &config.cookie_name).map(str::to_owned);
  let mut existing = None;
  if let Some(id) = &inbound
    && let Some(bytes) = store.load(id).await?
  {
    let record = serde_json::from_slice::<SessionRecord>(&bytes)?;
    if config
      .ttl
      .absolute_secs
      .is_none_or(|cap| now.saturating_sub(record.created_at) < cap)
    {
      existing = Some(record);
    } else {
      store.remove(id).await?;
    }
  }
  let was_existing = existing.is_some();
  let (id, record) = match (inbound, existing) {
    (Some(id), Some(record)) => (id, record),
    _ => (
      generate_session_id(),
      SessionRecord {
        data: serde_json::Map::new(),
        created_at: now,
      },
    ),
  };
  let session = Session::new(record.data);
  request.extensions_mut().insert(session.clone());
  let mut response = next.run(request).await;
  if session.is_destroyed() {
    if was_existing {
      store.remove(&id).await?;
    }
    let cookie = build_expired_cookie(
      &config.cookie_name,
      &config.path,
      config.domain.as_deref(),
      config.secure,
      config.http_only,
      config.same_site,
    );
    response
      .headers_mut()
      .append(http::header::SET_COOKIE, HeaderValue::from_str(&cookie)?);
    return Ok(response);
  }
  let effective_id = if session.rotation_requested() {
    if was_existing {
      store.remove(&id).await?;
    }
    generate_session_id()
  } else {
    id
  };
  let elapsed = SystemTime::now()
    .duration_since(UNIX_EPOCH)?
    .as_secs()
    .saturating_sub(record.created_at);
  let max_age = config
    .ttl
    .absolute_secs
    .map_or(config.ttl.idle_secs, |cap| {
      cap.saturating_sub(elapsed).min(config.ttl.idle_secs)
    });
  let bytes = serde_json::to_vec(&SessionRecord {
    data: session.snapshot(),
    created_at: record.created_at,
  })?;
  store
    .store(&effective_id, bytes, Duration::from_secs(max_age))
    .await?;
  let cookie = build_cookie(
    &config.cookie_name,
    &effective_id,
    &config.path,
    config.domain.as_deref(),
    max_age,
    config.secure,
    config.http_only,
    config.same_site,
  );
  response
    .headers_mut()
    .append(http::header::SET_COOKIE, HeaderValue::from_str(&cookie)?);
  Ok(response)
}
