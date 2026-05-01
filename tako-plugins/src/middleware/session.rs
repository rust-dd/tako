//! Cookie-based session middleware with in-memory store.
//!
//! Provides a session mechanism using cookies and an in-memory `scc::HashMap`
//! store. Sessions are identified by a random cookie value and support
//! get / set / remove operations for arbitrary `serde`-compatible types.
//!
//! v2 additions over the original middleware:
//!
//! - **Idle vs absolute timeout.** `idle_ttl` (default 1 h) bounds inactivity;
//!   `absolute_ttl` (default 24 h) bounds total session lifetime so a stolen
//!   session id cannot be refreshed forever.
//! - **Rolling cookie refresh.** Every dirty / touched session re-emits the
//!   `Set-Cookie` header with the refreshed `Max-Age`, not just the first
//!   request.
//! - **Privilege rotation.** [`Session::rotate`] swaps the underlying session
//!   id while keeping the data — defends against fixation after login.
//! - **Bulk revocation.** [`SessionMiddleware::handle`] returns a
//!   [`SessionStoreHandle`] with a `revoke_all` API for emergency purges.
//! - **`SameSite` selection.** Default stays `Lax`, but the builder accepts
//!   `Strict` or `None` (the latter requires `Secure` per browsers).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use http::HeaderValue;
use parking_lot::Mutex;
use scc::HashMap as SccHashMap;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tako_core::middleware::IntoMiddleware;
use tako_core::middleware::Next;
use tako_core::types::Request;
use tako_core::types::Response;

/// Session expiration policy.
#[derive(Clone, Copy)]
pub struct SessionTtl {
  /// Seconds of inactivity before the session is invalidated.
  pub idle_secs: u64,
  /// Hard cap on total session lifetime regardless of activity. `None` means
  /// only the idle timeout applies.
  pub absolute_secs: Option<u64>,
}

impl Default for SessionTtl {
  fn default() -> Self {
    Self {
      idle_secs: 3_600,
      absolute_secs: Some(86_400),
    }
  }
}

/// `SameSite` cookie attribute.
#[derive(Clone, Copy, Debug)]
pub enum SameSite {
  Strict,
  Lax,
  None,
}

impl SameSite {
  fn as_str(self) -> &'static str {
    match self {
      SameSite::Strict => "Strict",
      SameSite::Lax => "Lax",
      SameSite::None => "None",
    }
  }
}

#[derive(Clone)]
struct SessionEntry {
  data: serde_json::Map<String, serde_json::Value>,
  created_at: Instant,
  last_seen_at: Instant,
}

/// Internal session store wrapper. Cloneable handle to the same `SccHashMap`.
#[derive(Clone)]
struct Store(Arc<SccHashMap<String, SessionEntry>>);

impl Store {
  fn new() -> Self {
    Self(Arc::new(SccHashMap::new()))
  }

  fn get(&self, id: &str) -> Option<SessionEntry> {
    self.0.get_sync(id).map(|e| e.clone())
  }

  fn upsert(&self, id: String, entry: SessionEntry) {
    let _ = self.0.upsert_sync(id, entry);
  }

  fn remove(&self, id: &str) {
    let _ = self.0.remove_sync(id);
  }

  fn revoke_all(&self) {
    self.0.clear_sync();
  }

  fn revoke_predicate(&self, mut keep: impl FnMut(&str, &SessionEntry) -> bool) {
    self.0.retain_sync(|k, v| keep(k, v));
  }

  fn retain_expired(&self, ttl: SessionTtl) {
    let now = Instant::now();
    let idle = Duration::from_secs(ttl.idle_secs);
    let absolute = ttl.absolute_secs.map(Duration::from_secs);
    self.0.retain_sync(|_, v| {
      if now.duration_since(v.last_seen_at) > idle {
        return false;
      }
      if let Some(abs) = absolute {
        if now.duration_since(v.created_at) > abs {
          return false;
        }
      }
      true
    });
  }
}

/// Programmatic store handle returned by [`SessionMiddleware::handle`].
#[derive(Clone)]
pub struct SessionStoreHandle {
  store: Store,
}

impl SessionStoreHandle {
  /// Drops every session.
  pub fn revoke_all(&self) {
    self.store.revoke_all();
  }

  /// Drops sessions matching the predicate (returns false to drop).
  pub fn revoke_where<F>(&self, mut pred: F)
  where
    F: FnMut(&str, &serde_json::Map<String, serde_json::Value>) -> bool,
  {
    self
      .store
      .revoke_predicate(|k, v| !pred(k, &v.data));
  }
}

/// A session handle injected into request extensions.
#[derive(Clone)]
pub struct Session {
  data: Arc<Mutex<serde_json::Map<String, serde_json::Value>>>,
  dirty: Arc<AtomicBool>,
  rotation_counter: Arc<AtomicU64>,
}

impl Session {
  fn new(data: serde_json::Map<String, serde_json::Value>) -> Self {
    Self {
      data: Arc::new(Mutex::new(data)),
      dirty: Arc::new(AtomicBool::new(false)),
      rotation_counter: Arc::new(AtomicU64::new(0)),
    }
  }

  /// Reads a value from the session.
  pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
    self
      .data
      .lock()
      .get(key)
      .and_then(|v| serde_json::from_value(v.clone()).ok())
  }

  /// Stores a value in the session, marking it dirty.
  pub fn set<T: Serialize>(&self, key: &str, value: T) {
    if let Ok(v) = serde_json::to_value(value) {
      self.data.lock().insert(key.to_string(), v);
      self.dirty.store(true, Ordering::Relaxed);
    }
  }

  /// Removes a key from the session.
  pub fn remove(&self, key: &str) {
    if self.data.lock().remove(key).is_some() {
      self.dirty.store(true, Ordering::Relaxed);
    }
  }

  /// Empties the session keeping its id stable. Use this for logout flows
  /// where the cookie should still come back.
  pub fn clear(&self) {
    let mut guard = self.data.lock();
    if !guard.is_empty() {
      guard.clear();
      self.dirty.store(true, Ordering::Relaxed);
    }
  }

  /// Forces a fresh session id on the next response. Call this after
  /// privilege transitions (login / role change) to defend against
  /// fixation attacks.
  pub fn rotate(&self) {
    self.rotation_counter.fetch_add(1, Ordering::AcqRel);
    self.dirty.store(true, Ordering::Relaxed);
  }

  fn is_dirty(&self) -> bool {
    self.dirty.load(Ordering::Relaxed)
  }

  fn rotation_requested(&self) -> bool {
    self.rotation_counter.load(Ordering::Acquire) > 0
  }

  fn snapshot(&self) -> serde_json::Map<String, serde_json::Value> {
    self.data.lock().clone()
  }
}

/// Builder / configuration.
pub struct SessionMiddleware {
  cookie_name: String,
  ttl: SessionTtl,
  path: String,
  domain: Option<String>,
  secure: bool,
  http_only: bool,
  same_site: SameSite,
  store: Store,
}

impl Default for SessionMiddleware {
  fn default() -> Self {
    Self::new()
  }
}

impl SessionMiddleware {
  /// Creates a new session middleware with sensible defaults.
  pub fn new() -> Self {
    Self {
      cookie_name: "tako_session".to_string(),
      ttl: SessionTtl::default(),
      path: "/".to_string(),
      domain: None,
      secure: false,
      http_only: true,
      same_site: SameSite::Lax,
      store: Store::new(),
    }
  }

  /// Cookie name (default `"tako_session"`).
  pub fn cookie_name(mut self, name: &str) -> Self {
    self.cookie_name = name.to_string();
    self
  }

  /// Backwards-compatible idle TTL setter (sets `idle_secs`, leaves
  /// `absolute_secs` at the default 24 h cap).
  pub fn ttl_secs(mut self, secs: u64) -> Self {
    self.ttl.idle_secs = secs;
    self
  }

  /// Sets the full TTL policy.
  pub fn ttl(mut self, ttl: SessionTtl) -> Self {
    self.ttl = ttl;
    self
  }

  /// Cookie path (default `"/"`).
  pub fn path(mut self, path: &str) -> Self {
    self.path = path.to_string();
    self
  }

  /// Optional cookie `Domain` attribute.
  pub fn domain(mut self, domain: &str) -> Self {
    self.domain = Some(domain.to_string());
    self
  }

  /// Toggles the `Secure` flag.
  pub fn secure(mut self, secure: bool) -> Self {
    self.secure = secure;
    self
  }

  /// Toggles the `HttpOnly` flag (default true).
  pub fn http_only(mut self, on: bool) -> Self {
    self.http_only = on;
    self
  }

  /// Sets the `SameSite` attribute. Default: `Lax`. Note that `None` requires
  /// `Secure = true` per all major browsers.
  pub fn same_site(mut self, ss: SameSite) -> Self {
    self.same_site = ss;
    self
  }

  /// Returns a programmatic handle for revocation flows.
  pub fn handle(&self) -> SessionStoreHandle {
    SessionStoreHandle {
      store: self.store.clone(),
    }
  }
}

fn generate_session_id() -> String {
  uuid::Uuid::new_v4().simple().to_string()
}

fn extract_cookie_value<'a>(req: &'a Request, cookie_name: &str) -> Option<&'a str> {
  req
    .headers()
    .get(http::header::COOKIE)
    .and_then(|v| v.to_str().ok())
    .and_then(|cookies| {
      cookies.split(';').find_map(|pair| {
        let pair = pair.trim();
        let (name, value) = pair.split_once('=')?;
        if name.trim() == cookie_name {
          Some(value.trim())
        } else {
          None
        }
      })
    })
}

fn build_cookie(
  cookie_name: &str,
  sid: &str,
  path: &str,
  domain: Option<&str>,
  ttl_secs: u64,
  secure: bool,
  http_only: bool,
  same_site: SameSite,
) -> String {
  let mut s = format!("{}={}; Path={}", cookie_name, sid, path);
  if let Some(d) = domain {
    s.push_str("; Domain=");
    s.push_str(d);
  }
  s.push_str(&format!("; Max-Age={}", ttl_secs));
  if http_only {
    s.push_str("; HttpOnly");
  }
  if secure {
    s.push_str("; Secure");
  }
  s.push_str("; SameSite=");
  s.push_str(same_site.as_str());
  s
}

impl IntoMiddleware for SessionMiddleware {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let store = self.store.clone();
    let cookie_name = Arc::new(self.cookie_name);
    let ttl = self.ttl;
    let path = Arc::new(self.path);
    let domain = self.domain.map(Arc::new);
    let secure = self.secure;
    let http_only = self.http_only;
    let same_site = self.same_site;

    // Periodic janitor — expiry is enforced lazily on read, but a sweep keeps
    // RAM bounded for sessions that are never touched again.
    {
      let store = store.clone();
      let interval = Duration::from_secs(ttl.idle_secs.clamp(60, 3_600));
      #[cfg(not(feature = "compio"))]
      tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        loop {
          tick.tick().await;
          store.retain_expired(ttl);
        }
      });
      #[cfg(feature = "compio")]
      compio::runtime::spawn(async move {
        loop {
          compio::time::sleep(interval).await;
          store.retain_expired(ttl);
        }
      })
      .detach();
    }

    move |mut req: Request, next: Next| {
      let store = store.clone();
      let cookie_name = cookie_name.clone();
      let path = path.clone();
      let domain = domain.clone();

      Box::pin(async move {
        let now = Instant::now();
        let idle = Duration::from_secs(ttl.idle_secs);
        let absolute = ttl.absolute_secs.map(Duration::from_secs);

        let inbound_id = extract_cookie_value(&req, &cookie_name).map(str::to_string);
        let (sid, data, created_at, was_existing) = match inbound_id {
          Some(ref id) => match store.get(id) {
            Some(entry)
              if now.duration_since(entry.last_seen_at) <= idle
                && absolute
                  .map(|abs| now.duration_since(entry.created_at) <= abs)
                  .unwrap_or(true) =>
            {
              (id.clone(), entry.data, entry.created_at, true)
            }
            _ => {
              if let Some(id) = inbound_id.as_ref() {
                store.remove(id);
              }
              (
                generate_session_id(),
                serde_json::Map::new(),
                now,
                false,
              )
            }
          },
          None => (
            generate_session_id(),
            serde_json::Map::new(),
            now,
            false,
          ),
        };

        let session = Session::new(data);
        req.extensions_mut().insert(session.clone());

        let resp_outcome = next.run(req).await;
        let mut resp = resp_outcome;

        let dirty = session.is_dirty();
        let rotated = session.rotation_requested();

        // Effective session id: rotate if requested.
        let effective_sid = if rotated {
          if was_existing {
            store.remove(&sid);
          }
          generate_session_id()
        } else {
          sid
        };

        // Always touch on every request — rolling refresh keeps the cookie
        // alive while the user is active. Caller-side logout uses
        // `Session::clear` and the handler can pair it with explicit cookie
        // expiry by setting a `Set-Cookie` header itself.
        let updated_entry = SessionEntry {
          data: session.snapshot(),
          created_at,
          last_seen_at: now,
        };
        store.upsert(effective_sid.clone(), updated_entry);

        // Re-emit the cookie on every response. Browsers ignore identical
        // `Set-Cookie` headers cheaply; the upside is that long-lived UAs
        // see the refreshed `Max-Age`.
        let max_age = match absolute {
          Some(abs) => {
            let elapsed = now.duration_since(created_at);
            let absolute_remaining = abs.saturating_sub(elapsed);
            absolute_remaining.as_secs().min(idle.as_secs())
          }
          None => idle.as_secs(),
        };

        let cookie_value = build_cookie(
          &cookie_name,
          &effective_sid,
          &path,
          domain.as_deref().map(String::as_str),
          max_age,
          secure,
          http_only,
          same_site,
        );
        if let Ok(v) = HeaderValue::from_str(&cookie_value) {
          resp.headers_mut().append(http::header::SET_COOKIE, v);
        }

        // Suppress unused-variable warning on older toolchains; `dirty` is
        // intentionally read above to skip storage churn for unchanged
        // sessions in future when the API gains `if !dirty && !rotated`.
        let _ = dirty;

        resp
      })
    }
  }
}
