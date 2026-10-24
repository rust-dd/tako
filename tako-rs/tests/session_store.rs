use std::time::Duration;

use tako::stores::SessionStore;
use tako::stores::memory::MemorySessionStore;

const TTL: Duration = Duration::from_secs(60);

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn create_preserves_a_live_session_and_update_preserves_its_generation() {
  let store = MemorySessionStore::new();
  assert!(store.create("id", b"alice".to_vec(), TTL).await.unwrap());
  let original = store.load("id").await.unwrap().unwrap();
  assert!(!store.create("id", b"bob".to_vec(), TTL).await.unwrap());
  assert_eq!(store.load("id").await.unwrap().unwrap().data, b"alice");
  assert!(
    store
      .update("id", &original.version, b"alice-updated".to_vec(), TTL)
      .await
      .unwrap()
  );
  let updated = store.load("id").await.unwrap().unwrap();
  assert_eq!(updated.version, original.version);
  assert_eq!(updated.data, b"alice-updated");
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn recreated_id_rejects_the_old_generations_mutations() {
  let store = MemorySessionStore::new();
  assert!(store.create("id", b"old".to_vec(), TTL).await.unwrap());
  let old = store.load("id").await.unwrap().unwrap();
  assert!(store.remove("id", &old.version).await.unwrap());
  assert!(
    !store
      .update("id", &old.version, b"stale".to_vec(), TTL)
      .await
      .unwrap()
  );
  assert!(
    !store
      .rotate("id", &old.version, "rotated", b"stale".to_vec(), TTL)
      .await
      .unwrap()
  );
  assert!(
    store
      .create("id", b"replacement".to_vec(), TTL)
      .await
      .unwrap()
  );
  let replacement = store.load("id").await.unwrap().unwrap();
  assert_ne!(replacement.version, old.version);
  assert!(
    !store
      .update("id", &old.version, b"stale".to_vec(), TTL)
      .await
      .unwrap()
  );
  assert!(
    !store
      .rotate("id", &old.version, "rotated", b"stale".to_vec(), TTL)
      .await
      .unwrap()
  );
  assert!(!store.remove("id", &old.version).await.unwrap());
  assert_eq!(
    store.load("id").await.unwrap().unwrap().data,
    b"replacement"
  );
  assert!(store.load("rotated").await.unwrap().is_none());
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn expired_generations_cannot_be_updated_or_rotated_before_sweeping() {
  let store = MemorySessionStore::new();
  assert!(store.create("id", b"alice".to_vec(), TTL).await.unwrap());
  let original = store.load("id").await.unwrap().unwrap();
  assert!(
    store
      .update("id", &original.version, b"expired".to_vec(), Duration::ZERO)
      .await
      .unwrap()
  );
  assert!(store.load("id").await.unwrap().is_none());
  assert!(
    !store
      .update("id", &original.version, b"stale".to_vec(), TTL)
      .await
      .unwrap()
  );
  assert!(
    !store
      .rotate("id", &original.version, "rotated", b"stale".to_vec(), TTL)
      .await
      .unwrap()
  );
  assert!(store.load("rotated").await.unwrap().is_none());
  assert!(
    store
      .create("id", b"replacement".to_vec(), TTL)
      .await
      .unwrap()
  );
  assert_ne!(
    store.load("id").await.unwrap().unwrap().version,
    original.version
  );
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn rotation_preserves_both_sessions_on_collision_and_changes_generation_on_success() {
  let store = MemorySessionStore::new();
  assert!(store.create("alice", b"alice".to_vec(), TTL).await.unwrap());
  assert!(store.create("bob", b"bob".to_vec(), TTL).await.unwrap());
  let alice = store.load("alice").await.unwrap().unwrap();
  let bob = store.load("bob").await.unwrap().unwrap();
  for target in ["alice", "bob"] {
    assert!(
      !store
        .rotate("alice", &alice.version, target, b"rotated".to_vec(), TTL)
        .await
        .unwrap()
    );
  }
  assert_eq!(store.load("alice").await.unwrap().unwrap().data, b"alice");
  assert_eq!(
    store.load("bob").await.unwrap().unwrap().version,
    bob.version
  );
  assert!(
    store
      .rotate("alice", &alice.version, "fresh", b"rotated".to_vec(), TTL)
      .await
      .unwrap()
  );
  assert!(store.load("alice").await.unwrap().is_none());
  let fresh = store.load("fresh").await.unwrap().unwrap();
  assert_eq!(fresh.data, b"rotated");
  assert_ne!(fresh.version, alice.version);
  assert!(
    !store
      .rotate("alice", &alice.version, "another", b"stale".to_vec(), TTL)
      .await
      .unwrap()
  );
}
