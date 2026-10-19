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
  rotation_counter: Arc<AtomicU64>,
  destroyed: Arc<AtomicBool>,
}

impl Session {
  pub(crate) fn new(data: serde_json::Map<String, serde_json::Value>) -> Self {
    Self {
      data: Arc::new(Mutex::new(data)),
      rotation_counter: Arc::new(AtomicU64::new(0)),
      destroyed: Arc::new(AtomicBool::new(false)),
    }
  }

  /// `None` when the key is missing or does not deserialize as `T`.
  pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
    self
      .data
      .lock()
      .get(key)
      .and_then(|v| serde_json::from_value(v.clone()).ok())
  }

  /// A value that fails to serialize is dropped.
  pub fn set<T: Serialize>(&self, key: &str, value: T) {
    if let Ok(v) = serde_json::to_value(value) {
      self.data.lock().insert(key.to_string(), v);
    }
  }

  pub fn remove(&self, key: &str) {
    self.data.lock().remove(key);
  }

  /// Empties the data but keeps the id and cookie; log out with [`Self::destroy`].
  pub fn clear(&self) {
    self.data.lock().clear();
  }

  /// Removes the stored session and expires the cookie on this response.
  pub fn destroy(&self) {
    self.data.lock().clear();
    self.destroyed.store(true, Ordering::Release);
  }

  pub(crate) fn is_destroyed(&self) -> bool {
    self.destroyed.load(Ordering::Acquire)
  }

  /// Issues a new id on this response, keeping the data; call it after login or a role change
  /// to prevent session fixation.
  pub fn rotate(&self) {
    self.rotation_counter.fetch_add(1, Ordering::AcqRel);
  }

  /// Lets paired middleware such as CSRF mint fresh tokens alongside the rotated id.
  pub fn rotation_requested(&self) -> bool {
    self.rotation_counter.load(Ordering::Acquire) > 0
  }

  pub(crate) fn snapshot(&self) -> serde_json::Map<String, serde_json::Value> {
    self.data.lock().clone()
  }
}
