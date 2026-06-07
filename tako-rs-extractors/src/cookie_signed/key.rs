use cookie::Key;

/// Key ring for rotation-aware cookie signing/verification.
///
/// `active` is used to sign new cookies; `previous` keys are tried for
/// verification only, letting old cookies remain valid through a rotation.
/// Each key carries a string `kid` so callers can log which key admitted a
/// given cookie when [`CookieSigned::get_with_kid`](super::CookieSigned::get_with_kid) is used.
#[derive(Clone)]
pub struct KeyRing {
  pub(crate) active_kid: String,
  pub(crate) active: Key,
  pub(crate) previous: Vec<(String, Key)>,
}

impl KeyRing {
  /// Build a key ring with a single active key.
  pub fn new(active_kid: impl Into<String>, active: Key) -> Self {
    Self {
      active_kid: active_kid.into(),
      active,
      previous: Vec::new(),
    }
  }

  /// Add a previous key. Verification tries the active key first, then each
  /// previous key in insertion order.
  pub fn with_previous(mut self, kid: impl Into<String>, key: Key) -> Self {
    self.previous.push((kid.into(), key));
    self
  }

  /// Removes a previous key by `kid`. Cookies signed with that key will no
  /// longer be accepted — call this when a key has been disclosed or
  /// rotated past its retention window. Returns `true` if a key was removed.
  pub fn revoke(&mut self, kid: &str) -> bool {
    let before = self.previous.len();
    self.previous.retain(|(k, _)| k != kid);
    before != self.previous.len()
  }

  /// Returns the list of currently-trusted previous key ids in verification
  /// order. Use this to confirm a revocation took effect or to plan a key
  /// rotation.
  pub fn previous_kids(&self) -> impl Iterator<Item = &str> {
    self.previous.iter().map(|(k, _)| k.as_str())
  }

  /// Borrow the active key.
  pub fn active(&self) -> &Key {
    &self.active
  }

  /// The active key id.
  pub fn active_kid(&self) -> &str {
    &self.active_kid
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn keyring_revoke_removes_previous_key() {
    let active = Key::generate();
    let old = Key::generate();
    let older = Key::generate();
    let mut ring = KeyRing::new("v3", active)
      .with_previous("v2", old)
      .with_previous("v1", older);

    assert_eq!(ring.previous_kids().collect::<Vec<_>>(), vec!["v2", "v1"]);

    assert!(ring.revoke("v2"));
    assert_eq!(ring.previous_kids().collect::<Vec<_>>(), vec!["v1"]);

    // No-op for non-existent kid.
    assert!(!ring.revoke("v99"));

    assert!(ring.revoke("v1"));
    assert_eq!(ring.previous_kids().count(), 0);
  }
}
