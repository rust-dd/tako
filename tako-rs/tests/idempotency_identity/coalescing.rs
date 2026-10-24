use tako::stores::memory::MemoryIdempotencyStore;
use tokio::sync::Notify;

use super::*;

fn waiting_router(
  store: MemoryIdempotencyStore,
  calls: Arc<AtomicUsize>,
  release: Arc<Notify>,
) -> Router {
  let mut router = Router::new();
  router.middleware(authenticate);
  router.plugin(IdempotencyBuilder::new().store(store).build());
  router.post("/charge", move |request: Request| {
    calls.fetch_add(1, Ordering::SeqCst);
    let principal = request.extensions().get::<Principal>().unwrap().clone();
    let release = release.clone();
    async move {
      release.notified().await;
      format!(
        "private receipt for {}/{}",
        principal.tenant, principal.user
      )
    }
  });
  router
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn only_same_identity_requests_coalesce_across_routers() {
  let store = MemoryIdempotencyStore::new();
  let calls = Arc::new(AtomicUsize::new(0));
  let release = Arc::new(Notify::new());
  let first = waiting_router(store.clone(), calls.clone(), release.clone());
  let second = waiting_router(store, calls.clone(), release.clone());

  let mut alice = std::pin::pin!(first.dispatch(post("alice")));
  assert!(futures_util::poll!(&mut alice).is_pending());
  let mut alice_retry = std::pin::pin!(second.dispatch(post("alice-refreshed")));
  assert!(futures_util::poll!(&mut alice_retry).is_pending());
  assert_eq!(calls.load(Ordering::SeqCst), 1);

  let mut bob = std::pin::pin!(second.dispatch(post("bob")));
  assert!(futures_util::poll!(&mut bob).is_pending());
  assert_eq!(calls.load(Ordering::SeqCst), 2);
  let mut alice_south = std::pin::pin!(second.dispatch(post("alice-south")));
  assert!(futures_util::poll!(&mut alice_south).is_pending());
  assert_eq!(calls.load(Ordering::SeqCst), 3);

  release.notify_waiters();
  let (alice, alice_retry, bob, alice_south) =
    futures_util::join!(alice, alice_retry, bob, alice_south);
  assert_eq!(body(alice).await, "private receipt for north/alice");
  assert_eq!(body(alice_retry).await, "private receipt for north/alice");
  assert_eq!(body(bob).await, "private receipt for north/bob");
  assert_eq!(body(alice_south).await, "private receipt for south/alice");
  assert_eq!(calls.load(Ordering::SeqCst), 3);
}
