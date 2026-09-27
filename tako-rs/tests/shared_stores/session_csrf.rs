use std::time::Duration;

use tako::middleware::csrf::Csrf;
use tako::middleware::session::Session;
use tako::middleware::session::SessionMiddleware;
use tako::stores::CsrfTokenStore;
use tako::stores::memory::MemoryCsrfTokenStore;
use tako::stores::memory::MemorySessionStore;

use super::*;

fn session_router(store: MemorySessionStore) -> Router {
  let mut router = Router::new();
  router.middleware(SessionMiddleware::new().store(store).into_middleware());
  router.get("/", |req: Request| async move {
    let session = req.extensions().get::<Session>().unwrap();
    let count = session.get::<u32>("count").unwrap_or(0) + 1;
    session.set("count", count);
    count.to_string()
  });
  router.post("/rotate", |req: Request| async move {
    req.extensions().get::<Session>().unwrap().rotate();
    "rotated"
  });
  router.post("/logout", |req: Request| async move {
    req.extensions().get::<Session>().unwrap().destroy();
    "bye"
  });
  router
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn session_data_rotation_and_revocation_cross_router_boundaries() {
  let store = MemorySessionStore::new();
  let first = session_router(store.clone());
  let second = session_router(store);
  let response = first.dispatch(request(Method::GET, "/", "")).await;
  let original = cookies(&response);
  assert_eq!(body(response).await, "1");
  assert_eq!(
    body(second.dispatch(request(Method::GET, "/", &original)).await).await,
    "2"
  );
  let rotated = first
    .dispatch(request(Method::POST, "/rotate", &original))
    .await;
  let rotated = cookies(&rotated);
  assert_ne!(original, rotated);
  assert_eq!(
    body(second.dispatch(request(Method::GET, "/", &rotated)).await).await,
    "3"
  );
  assert_eq!(
    body(second.dispatch(request(Method::GET, "/", &original)).await).await,
    "1"
  );
  let logout = first
    .dispatch(request(Method::POST, "/logout", &rotated))
    .await;
  assert!(
    logout.headers()["set-cookie"]
      .to_str()
      .unwrap()
      .contains("Max-Age=0")
  );
  assert_eq!(
    body(second.dispatch(request(Method::GET, "/", &rotated)).await).await,
    "1"
  );
}

fn csrf_router(sessions: MemorySessionStore, tokens: MemoryCsrfTokenStore) -> Router {
  let mut router = Router::new();
  router.middleware(SessionMiddleware::new().store(sessions).into_middleware());
  router.middleware(Csrf::new().store(tokens).single_use(true).into_middleware());
  router.get("/", || async { "ready" });
  router.post("/", || async { "accepted" });
  router
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn csrf_tokens_are_shared_and_consumed_once() {
  let sessions = MemorySessionStore::new();
  let tokens = MemoryCsrfTokenStore::new();
  let first = csrf_router(sessions.clone(), tokens.clone());
  let second = csrf_router(sessions, tokens);
  let response = first.dispatch(request(Method::GET, "/", "")).await;
  assert_eq!(response.status(), StatusCode::OK);
  let initial = cookies(&response);
  let token = initial
    .split("; ")
    .find_map(|part| part.strip_prefix("csrf_token="))
    .unwrap();
  let post = || {
    let mut request = request(Method::POST, "/", &initial);
    request
      .headers_mut()
      .insert("x-csrf-token", token.parse().unwrap());
    request
  };
  let accepted = second.dispatch(post()).await;
  assert_eq!(accepted.status(), StatusCode::OK);
  assert_ne!(cookies(&accepted), initial);
  assert_eq!(first.dispatch(post()).await.status(), StatusCode::FORBIDDEN);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn concurrent_single_use_and_expired_tokens_are_rejected() {
  let store = MemoryCsrfTokenStore::new();
  let token = store
    .issue("session", Duration::from_secs(60))
    .await
    .unwrap();
  let results =
    futures_util::future::join_all((0..32).map(|_| store.validate("session", &token, true))).await;
  assert_eq!(
    results
      .into_iter()
      .filter(|result| matches!(result, Ok(true)))
      .count(),
    1
  );
  assert!(!store.validate("session", &token, false).await.unwrap());
  let token = store.issue("session", Duration::ZERO).await.unwrap();
  assert!(!store.validate("session", &token, false).await.unwrap());
}

#[cfg(feature = "plugins")]
#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn rate_quota_is_shared_and_headers_use_the_backend_capacity() {
  use tako::plugins::rate_limiter::RateLimiterBuilder;
  use tako::stores::memory::MemoryRateLimitStore;
  let store = MemoryRateLimitStore::new(1, 0.0001);
  let make_router = || {
    let mut router = Router::new();
    router.plugin(
      RateLimiterBuilder::new()
        .max_requests(99)
        .key_fn(|_| Some("account".into()))
        .store(store.clone())
        .build(),
    );
    router.get("/", || async { "ok" });
    router
  };
  let first = make_router();
  let second = make_router();
  let response = first.dispatch(request(Method::GET, "/", "")).await;
  assert_eq!(response.status(), StatusCode::OK);
  assert_eq!(response.headers()["ratelimit-limit"], "1");
  let response = second.dispatch(request(Method::GET, "/", "")).await;
  assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
  assert_eq!(response.headers()["ratelimit-remaining"], "0");
}
