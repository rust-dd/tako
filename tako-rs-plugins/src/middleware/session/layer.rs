//! The [`SessionMiddleware`] builder and its request-time enforcement.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

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
use super::store::SessionEntry;
use super::store::SessionStoreHandle;
use super::store::SessionTtl;
use super::store::Store;

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
                && absolute.is_none_or(|abs| now.duration_since(entry.created_at) <= abs) =>
            {
              (id.clone(), entry.data, entry.created_at, true)
            }
            _ => {
              if let Some(id) = inbound_id.as_ref() {
                store.remove(id);
              }
              (generate_session_id(), serde_json::Map::new(), now, false)
            }
          },
          None => (generate_session_id(), serde_json::Map::new(), now, false),
        };

        let session = Session::new(data);
        req.extensions_mut().insert(session.clone());

        let resp_outcome = next.run(req).await;
        let mut resp = resp_outcome;

        let dirty = session.is_dirty();
        let rotated = session.rotation_requested();
        let destroyed = session.is_destroyed();

        // Destruction (logout) takes precedence over rotation/refresh: drop
        // the server entry and emit a Set-Cookie that the UA will treat as
        // an immediate delete.
        if destroyed {
          if was_existing {
            store.remove(&sid);
          }
          let expired = build_expired_cookie(
            &cookie_name,
            &path,
            domain.as_deref().map(String::as_str),
            secure,
            http_only,
            same_site,
          );
          if let Ok(v) = HeaderValue::from_str(&expired) {
            resp.headers_mut().append(http::header::SET_COOKIE, v);
          }
          let _ = dirty;
          return resp;
        }

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
        // `Session::destroy` which short-circuits this path.
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

        let _ = dirty;

        resp
      })
    }
  }
}
