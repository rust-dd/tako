use cookie::Key;

/// Keys for rotation: the active key signs and encrypts, previous keys only verify and decrypt,
/// so cookies issued before a rotation stay valid until their key is revoked.
#[derive(Clone)]
pub struct KeyRing {
  pub(crate) active_kid: String,
  pub(crate) active: Key,
  pub(crate) previous: Vec<(String, Key)>,
}

impl KeyRing {
  pub fn new(active_kid: impl Into<String>, active: Key) -> Self {
    Self {
      active_kid: active_kid.into(),
      active,
      previous: Vec::new(),
    }
  }

  /// Previous keys are tried in insertion order, after the active key.
  pub fn with_previous(mut self, kid: impl Into<String>, key: Key) -> Self {
    self.previous.push((kid.into(), key));
    self
  }

  /// Drops a previous key so its cookies stop verifying; returns whether `kid` was present.
  pub fn revoke(&mut self, kid: &str) -> bool {
    let before = self.previous.len();
    self.previous.retain(|(k, _)| k != kid);
    before != self.previous.len()
  }

  /// Previous key ids, in the order they are tried.
  pub fn previous_kids(&self) -> impl Iterator<Item = &str> {
    self.previous.iter().map(|(k, _)| k.as_str())
  }

  pub fn active(&self) -> &Key {
    &self.active
  }

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

    assert!(!ring.revoke("v99"));

    assert!(ring.revoke("v1"));
    assert_eq!(ring.previous_kids().count(), 0);
  }
}
