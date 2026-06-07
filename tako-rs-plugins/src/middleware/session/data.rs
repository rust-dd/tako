//! The per-request [`Session`] handle injected into request extensions.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use parking_lot::Mutex;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// A session handle injected into request extensions.
#[derive(Clone)]
pub struct Session {
  data: Arc<Mutex<serde_json::Map<String, serde_json::Value>>>,
  dirty: Arc<AtomicBool>,
  rotation_counter: Arc<AtomicU64>,
  destroyed: Arc<AtomicBool>,
}

impl Session {
  pub(crate) fn new(data: serde_json::Map<String, serde_json::Value>) -> Self {
    Self {
      data: Arc::new(Mutex::new(data)),
      dirty: Arc::new(AtomicBool::new(false)),
      rotation_counter: Arc::new(AtomicU64::new(0)),
      destroyed: Arc::new(AtomicBool::new(false)),
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

  /// Empties the session keeping its id stable. Use this when you want the
  /// session to live on (e.g. clearing temporary state) but the cookie should
  /// keep being refreshed. For logout flows that should remove the cookie
  /// from the browser, use [`Self::destroy`] instead.
  pub fn clear(&self) {
    let mut guard = self.data.lock();
    if !guard.is_empty() {
      guard.clear();
      self.dirty.store(true, Ordering::Relaxed);
    }
  }

  /// Marks the session for destruction: the server-side entry is removed and
  /// the response Set-Cookie carries `Max-Age=0` with a past `Expires` so the
  /// user agent drops it. Pair this with whatever logout response your
  /// application returns.
  pub fn destroy(&self) {
    self.data.lock().clear();
    self.destroyed.store(true, Ordering::Release);
    self.dirty.store(true, Ordering::Relaxed);
  }

  pub(crate) fn is_destroyed(&self) -> bool {
    self.destroyed.load(Ordering::Acquire)
  }

  /// Forces a fresh session id on the next response. Call this after
  /// privilege transitions (login / role change) to defend against
  /// fixation attacks.
  pub fn rotate(&self) {
    self.rotation_counter.fetch_add(1, Ordering::AcqRel);
    self.dirty.store(true, Ordering::Relaxed);
  }

  pub(crate) fn is_dirty(&self) -> bool {
    self.dirty.load(Ordering::Relaxed)
  }

  /// True if [`Session::rotate`] has been called on this handle since the
  /// session middleware created it. Surfaced as public API so paired
  /// middleware (notably CSRF) can mint fresh derivative tokens on the same
  /// response that emits the rotated session id.
  pub fn rotation_requested(&self) -> bool {
    self.rotation_counter.load(Ordering::Acquire) > 0
  }

  pub(crate) fn snapshot(&self) -> serde_json::Map<String, serde_json::Value> {
    self.data.lock().clone()
  }
}
