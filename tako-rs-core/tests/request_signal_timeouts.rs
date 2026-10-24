#![cfg(feature = "signals")]

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use http::StatusCode;
use tako_rs_core::router::Router;
use tako_rs_core::signals::ids;
use tako_rs_core::types::Request;

struct Dropped(Arc<AtomicUsize>);

impl Drop for Dropped {
  fn drop(&mut self) {
    self.0.fetch_add(1, Ordering::SeqCst);
  }
}

async fn bounded<T>(future: impl Future<Output = T>) -> T {
  let work = std::pin::pin!(future);
  let deadline = std::pin::pin!(sleep(Duration::from_secs(1)));
  match futures_util::future::select(work, deadline).await {
    futures_util::future::Either::Left((value, _)) => value,
    futures_util::future::Either::Right(_) => panic!("request escaped its configured timeout"),
  }
}

async fn sleep(duration: Duration) {
  #[cfg(not(feature = "compio"))]
  tokio::time::sleep(duration).await;
  #[cfg(feature = "compio")]
  send_wrapper::SendWrapper::new(compio::time::sleep(duration)).await;
}

async fn pending_hook_times_out(id: &'static str) {
  let mut router = Router::new();
  router.timeout(Duration::from_millis(10));
  let calls = Arc::new(AtomicUsize::new(0));
  let drops = Arc::new(AtomicUsize::new(0));
  let handler_calls = Arc::new(AtomicUsize::new(0));
  let observed = calls.clone();
  let resources = drops.clone();
  router.on_signal(id, move |_| {
    observed.fetch_add(1, Ordering::SeqCst);
    let guard = Dropped(resources.clone());
    async move {
      let _guard = guard;
      std::future::pending::<()>().await;
    }
  });
  let handler_observed = handler_calls.clone();
  router.get("/", move || {
    handler_observed.fetch_add(1, Ordering::SeqCst);
    async { "ok" }
  });
  let mut completion = router.signals().subscribe(ids::REQUEST_COMPLETED);
  let mut route_completion = router.signals().subscribe(ids::ROUTE_REQUEST_COMPLETED);
  let response = bounded(router.dispatch(Request::default())).await;
  assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
  assert_eq!(calls.load(Ordering::SeqCst), 1);
  assert_eq!(drops.load(Ordering::SeqCst), 1);
  let started = id == ids::REQUEST_STARTED || id == ids::ROUTE_REQUEST_STARTED;
  assert_eq!(handler_calls.load(Ordering::SeqCst), usize::from(!started));
  assert!(std::iter::from_fn(|| completion.try_recv().ok()).count() <= 1);
  assert!(std::iter::from_fn(|| route_completion.try_recv().ok()).count() <= 1);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn pending_started_hooks_are_cancelled_at_the_request_deadline() {
  for id in [ids::REQUEST_STARTED, ids::ROUTE_REQUEST_STARTED] {
    pending_hook_times_out(id).await;
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn pending_completed_hooks_are_cancelled_without_replaying_completion() {
  for id in [ids::REQUEST_COMPLETED, ids::ROUTE_REQUEST_COMPLETED] {
    pending_hook_times_out(id).await;
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn cancelling_dispatch_drops_the_pending_hook() {
  let router = Router::new();
  let drops = Arc::new(AtomicUsize::new(0));
  let resources = drops.clone();
  router.on_signal(ids::REQUEST_STARTED, move |_| {
    let guard = Dropped(resources.clone());
    async move {
      let _guard = guard;
      std::future::pending::<()>().await;
    }
  });
  let mut request = Box::pin(router.dispatch(Request::default()));
  assert!(futures_util::poll!(&mut request).is_pending());
  drop(request);
  assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn hooks_share_the_remaining_request_budget() {
  let mut router = Router::new();
  router.timeout(Duration::from_millis(75));
  for id in [ids::REQUEST_STARTED, ids::REQUEST_COMPLETED] {
    router.on_signal(id, |_| async {
      sleep(Duration::from_millis(30)).await;
    });
  }
  router.get("/", || async {
    sleep(Duration::from_millis(30)).await;
    "ok"
  });
  let response = bounded(router.dispatch(Request::default())).await;
  assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn hook_timeout_keeps_nested_fallback_metadata_and_head_semantics() {
  let mut child = Router::new();
  child.timeout(Duration::from_millis(10));
  child.timeout_fallback(|request: Request| async move {
    assert_eq!(request.method(), http::Method::HEAD);
    assert_eq!(request.uri(), "/api/slow?key=value");
    assert_eq!(request.headers()["x-request"], "metadata");
    (StatusCode::GATEWAY_TIMEOUT, "timed out")
  });
  child.get("/slow", || async { "ok" });
  let mut router = Router::new();
  router.on_signal(ids::REQUEST_STARTED, |_| std::future::pending());
  router.nest("/api", child);
  let request = http::Request::builder()
    .method("HEAD")
    .uri("/api/slow?key=value")
    .header("x-request", "metadata")
    .body(tako_rs_core::body::TakoBody::empty())
    .unwrap();
  let response = bounded(router.dispatch(request)).await;
  assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
  assert_eq!(response.headers()[http::header::CONTENT_LENGTH], "9");
  assert!(
    http_body_util::BodyExt::collect(response.into_body())
      .await
      .unwrap()
      .to_bytes()
      .is_empty()
  );
}
