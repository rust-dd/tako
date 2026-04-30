//! Cookie-based session middleware with in-memory store.
//!
//! Provides a simple session mechanism using cookies and an in-memory `scc::HashMap` store.
//! Sessions are identified by a random cookie value and support get/set/remove operations
//! for arbitrary `serde`-compatible types.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use http::HeaderValue;
use scc::HashMap as SccHashMap;
use serde::Serialize;
use serde::de::DeserializeOwned;

use tako_core::middleware::IntoMiddleware;
use tako_core::middleware::Next;
use tako_core::types::Request;
use tako_core::types::Response;

/// Session data stored in memory.
#[derive(Clone)]
struct SessionEntry {
  data: serde_json::Map<String, serde_json::Value>,
  expires_at: Instant,
}

/// Session store backed by `scc::HashMap`.
#[derive(Clone)]
struct Store(Arc<SccHashMap<String, SessionEntry>>);

impl Store {
  fn new() -> Self {
    Self(Arc::new(SccHashMap::new()))
  }

  fn get(&self, id: &str) -> Option<SessionEntry> {
    self.0.get_sync(id).map(|e| e.clone())
  }

  fn set(&self, id: String, entry: SessionEntry) {
    let _ = self.0.insert_sync(id, entry);
  }

  fn retain_expired(&self) {
    let now = Instant::now();
    self.0.retain_sync(|_, v| v.expires_at > now);
  }
}

/// A session handle injected into request extensions.
///
/// Use this to get/set/remove values from the current session.
#[derive(Clone)]
pub struct Session {
  data: Arc<parking_lot::Mutex<serde_json::Map<String, serde_json::Value>>>,
  dirty: Arc<std::sync::atomic::AtomicBool>,
}

impl Session {
  fn new(data: serde_json::Map<String, serde_json::Value>) -> Self {
    Self {
      data: Arc::new(parking_lot::Mutex::new(data)),
      dirty: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    }
  }

  /// Gets a value from the session.
  pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
    let data = self.data.lock();
    data.get(key).and_then(|v| serde_json::from_value(v.clone()).ok())
  }

  /// Sets a value in the session.
  pub fn set<T: Serialize>(&self, key: &str, value: T) {
    if let Ok(v) = serde_json::to_value(value) {
      let mut data = self.data.lock();
      data.insert(key.to_string(), v);
      self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
    }
  }

  /// Removes a value from the session.
  pub fn remove(&self, key: &str) {
    let mut data = self.data.lock();
    if data.remove(key).is_some() {
      self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
    }
  }

  fn is_dirty(&self) -> bool {
    self.dirty.load(std::sync::atomic::Ordering::Relaxed)
  }

  fn into_map(self) -> serde_json::Map<String, serde_json::Value> {
    Arc::try_unwrap(self.data)
      .map(|m| m.into_inner())
      .unwrap_or_else(|arc| arc.lock().clone())
  }
}

/// Session middleware configuration.
///
/// # Examples
///
/// ```rust
/// use tako::middleware::session::SessionMiddleware;
/// use tako::middleware::IntoMiddleware;
///
/// let session = SessionMiddleware::new()
///     .cookie_name("my_session")
///     .ttl_secs(3600);
/// let mw = session.into_middleware();
/// ```
pub struct SessionMiddleware {
  cookie_name: String,
  ttl_secs: u64,
  path: String,
  secure: bool,
  http_only: bool,
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
      ttl_secs: 3600,
      path: "/".to_string(),
      secure: false,
      http_only: true,
    }
  }

  /// Sets the session cookie name.
  pub fn cookie_name(mut self, name: &str) -> Self {
    self.cookie_name = name.to_string();
    self
  }

  /// Sets the session TTL in seconds.
  pub fn ttl_secs(mut self, secs: u64) -> Self {
    self.ttl_secs = secs;
    self
  }

  /// Sets the cookie path.
  pub fn path(mut self, path: &str) -> Self {
    self.path = path.to_string();
    self
  }

  /// Sets the Secure flag on the cookie.
  pub fn secure(mut self, secure: bool) -> Self {
    self.secure = secure;
    self
  }
}

fn generate_session_id() -> String {
  use std::time::{SystemTime, UNIX_EPOCH};
  let now = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_nanos();
  let a = now.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
  let b = a.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
  format!("{:032x}{:032x}", a, b)
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

impl IntoMiddleware for SessionMiddleware {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let store = Store::new();
    let cookie_name = Arc::new(self.cookie_name);
    let ttl_secs = self.ttl_secs;
    let path = Arc::new(self.path);
    let secure = self.secure;
    let http_only = self.http_only;

    // Start cleanup janitor
    {
      let store = store.clone();
      let interval = Duration::from_secs(ttl_secs.max(60).min(3600));
      #[cfg(not(feature = "compio"))]
      tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        loop {
          tick.tick().await;
          store.retain_expired();
        }
      });
      #[cfg(feature = "compio")]
      compio::runtime::spawn(async move {
        loop {
          compio::time::sleep(interval).await;
          store.retain_expired();
        }
      })
      .detach();
    }

    move |mut req: Request, next: Next| {
      let store = store.clone();
      let cookie_name = cookie_name.clone();
      let path = path.clone();

      Box::pin(async move {
        // Extract session ID from cookie
        let session_id = extract_cookie_value(&req, &cookie_name).map(|s| s.to_string());

        let (sid, session_data) = if let Some(ref id) = session_id {
          if let Some(entry) = store.get(id) {
            if entry.expires_at > Instant::now() {
              (id.clone(), entry.data)
            } else {
              let new_id = generate_session_id();
              (new_id, serde_json::Map::new())
            }
          } else {
            let new_id = generate_session_id();
            (new_id, serde_json::Map::new())
          }
        } else {
          let new_id = generate_session_id();
          (new_id, serde_json::Map::new())
        };

        let is_new = session_id.as_ref() != Some(&sid);
        let session = Session::new(session_data);
        req.extensions_mut().insert(session.clone());

        let mut resp = next.run(req).await;

        // Save session if dirty or new
        if session.is_dirty() || is_new {
          let data = session.into_map();
          store.set(
            sid.clone(),
            SessionEntry {
              data,
              expires_at: Instant::now() + Duration::from_secs(ttl_secs),
            },
          );
        }

        // Set cookie if new session
        if is_new {
          let mut cookie_str = format!("{}={}; Path={}", cookie_name, sid, path);
          cookie_str.push_str(&format!("; Max-Age={}", ttl_secs));
          if http_only {
            cookie_str.push_str("; HttpOnly");
          }
          if secure {
            cookie_str.push_str("; Secure");
          }
          cookie_str.push_str("; SameSite=Lax");
          if let Ok(val) = HeaderValue::from_str(&cookie_str) {
            resp.headers_mut().append(http::header::SET_COOKIE, val);
          }
        }

        resp
      })
    }
  }
}
