use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tako::plugins::idempotency::IdempotencyBuilder;
use tako::plugins::idempotency::Scope;
use tako::stores::IdempotencyBegin;
use tako::stores::IdempotencyEntry;
use tako::stores::IdempotencyStore;
use tako::stores::memory::MemoryIdempotencyStore;
use tokio::sync::Notify;

use super::*;

fn post(payload: &'static str) -> Request {
  http::Request::builder()
    .method(Method::POST)
    .uri("/")
    .header("idempotency-key", "operation")
    .body(TakoBody::from(payload))
    .unwrap()
}

fn router(store: MemoryIdempotencyStore, calls: Arc<AtomicUsize>, release: Arc<Notify>) -> Router {
  let mut router = Router::new();
  router.plugin(
    IdempotencyBuilder::new()
      .store(store)
      .scope(Scope::KeyOnly)
      .build(),
  );
  router.post("/", move || {
    let calls = calls.clone();
    let release = release.clone();
    async move {
      calls.fetch_add(1, Ordering::SeqCst);
      release.notified().await;
      http::Response::builder()
        .status(StatusCode::CREATED)
        .header("x-result", "created")
        .header("set-cookie", "private=secret")
        .body(TakoBody::from("result"))
        .unwrap()
    }
  });
  router
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn requests_on_two_routers_coalesce_and_replay_without_private_cookies() {
  let store = MemoryIdempotencyStore::new();
  let calls = Arc::new(AtomicUsize::new(0));
  let release = Arc::new(Notify::new());
  let first = router(store.clone(), calls.clone(), release.clone());
  let second = router(store, calls.clone(), release.clone());
  let mut owner = std::pin::pin!(first.dispatch(post("body")));
  assert!(futures_util::poll!(&mut owner).is_pending());
  let mut duplicate = std::pin::pin!(second.dispatch(post("body")));
  assert!(futures_util::poll!(&mut duplicate).is_pending());
  assert_eq!(calls.load(Ordering::SeqCst), 1);
  assert_eq!(
    second.dispatch(post("different")).await.status(),
    StatusCode::CONFLICT
  );
  release.notify_one();
  let (original, replay) = futures_util::join!(owner, duplicate);
  assert_eq!(original.status(), StatusCode::CREATED);
  assert!(original.headers().contains_key("set-cookie"));
  assert_eq!(replay.status(), StatusCode::CREATED);
  assert_eq!(replay.headers()["x-result"], "created");
  assert!(!replay.headers().contains_key("set-cookie"));
  assert_eq!(body(replay).await, "result");
  assert_eq!(body(first.dispatch(post("body")).await).await, "result");
  assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn oversized_and_unknown_length_responses_keep_the_original_body() {
  for streamed in [false, true] {
    let calls = Arc::new(AtomicUsize::new(0));
    let handler_calls = calls.clone();
    let mut router = Router::new();
    router.plugin(IdempotencyBuilder::new().max_cached_body_bytes(2).build());
    router.post("/", move || {
      handler_calls.fetch_add(1, Ordering::SeqCst);
      async move {
        let body = if streamed {
          TakoBody::from_stream(futures_util::stream::iter([
            Ok::<_, tako::types::BoxError>(bytes::Bytes::from_static(b"oversized")),
          ]))
        } else {
          TakoBody::from("oversized")
        };
        Response::new(body)
      }
    });
    for _ in 0..2 {
      assert_eq!(body(router.dispatch(post("body")).await).await, "oversized");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn expired_owners_cannot_complete_or_remove_new_leases() {
  let store = MemoryIdempotencyStore::new().with_inflight_ttl(Duration::ZERO);
  let IdempotencyBegin::Acquired(old) = store.begin("key", [1; 32]).await.unwrap() else {
    panic!("owner");
  };
  assert!(store.get("key").await.unwrap().is_none());
  let store = store.with_inflight_ttl(Duration::from_secs(60));
  let IdempotencyBegin::Acquired(new) = store.begin("key", [2; 32]).await.unwrap() else {
    panic!("new owner");
  };
  let entry = IdempotencyEntry {
    status: 200,
    headers: vec![],
    body: bytes::Bytes::from_static(b"new"),
    payload_sig: [2; 32],
    completed: true,
  };
  assert!(!store.remove("key", &old).await.unwrap());
  assert!(
    !store
      .complete("key", &old, entry.clone(), Duration::from_secs(60))
      .await
      .unwrap()
  );
  assert!(
    store
      .complete("key", &new, entry, Duration::from_secs(60))
      .await
      .unwrap()
  );
  assert_eq!(store.get("key").await.unwrap().unwrap().body, "new");
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn cancelling_an_owner_releases_its_lease() {
  let store = MemoryIdempotencyStore::new();
  let first = router(
    store.clone(),
    Arc::new(AtomicUsize::new(0)),
    Arc::new(Notify::new()),
  );
  let mut owner = Box::pin(first.dispatch(post("body")));
  assert!(futures_util::poll!(&mut owner).is_pending());
  drop(owner);
  let entry = store
    .wait("operation", Some(Duration::from_secs(1)))
    .await
    .unwrap();
  assert!(entry.is_none());
}
