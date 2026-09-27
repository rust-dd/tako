#![cfg(feature = "plugins")]

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use tako::StatusCode;
use tako::body::TakoBody;
use tako::plugins::TakoPlugin;
use tako::router::Router;
use tako::types::Request;

#[derive(Clone)]
struct HeaderPlugin(Arc<AtomicUsize>);

impl TakoPlugin for HeaderPlugin {
  fn name(&self) -> &'static str {
    "header"
  }
  fn setup(&self, router: &Router) -> anyhow::Result<()> {
    self.0.fetch_add(1, Ordering::SeqCst);
    router.middleware(|req, next| async move {
      let mut response = next.run(req).await;
      response
        .headers_mut()
        .append("x-plugin", "yes".parse().unwrap());
      response
    });
    Ok(())
  }
}

#[derive(Clone)]
struct FailingPlugin;

impl TakoPlugin for FailingPlugin {
  fn name(&self) -> &'static str {
    "broken-security-plugin"
  }
  fn setup(&self, _: &Router) -> anyhow::Result<()> {
    anyhow::bail!("configuration unavailable")
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn nested_router_and_route_plugins_are_applied_once() {
  let calls = Arc::new(AtomicUsize::new(0));
  let mut child = Router::new();
  child.plugin(HeaderPlugin(calls.clone()));
  child
    .get("/x", || async { "ok" })
    .plugin(HeaderPlugin(calls.clone()));
  let mut root = Router::new();
  root.nest("/api", child);
  for _ in 0..2 {
    let req = http::Request::builder()
      .uri("/api/x")
      .body(TakoBody::empty())
      .unwrap();
    let response = root.dispatch(req).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get_all("x-plugin").iter().count(), 2);
  }
  assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn failed_plugins_prevent_handlers_from_running() {
  for route_level in [false, true] {
    let mut router = Router::new();
    if !route_level {
      router.plugin(FailingPlugin);
    }
    let route = router.get("/", || async {
      panic!("must fail closed");
      #[allow(unreachable_code)]
      "secret"
    });
    if route_level {
      route.plugin(FailingPlugin);
    }
    for _ in 0..2 {
      assert_eq!(
        router.dispatch(Request::default()).await.status(),
        StatusCode::INTERNAL_SERVER_ERROR
      );
    }
  }
}

#[cfg(feature = "signals")]
#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn router_listeners_observe_route_labels_and_duration() {
  use tako::signals::ids;
  let mut router = Router::new();
  router.get("/users/{id}", || async { "ok" });
  let mut completed = router.signals().subscribe(ids::REQUEST_COMPLETED);
  let mut route_completed = router.signals().subscribe(ids::ROUTE_REQUEST_COMPLETED);
  let mut app_completed = tako::signals::app_signals().subscribe(ids::ROUTE_REQUEST_COMPLETED);
  let req = http::Request::builder()
    .uri("/users/42")
    .body(TakoBody::empty())
    .unwrap();
  router.dispatch(req).await;
  let app_signal = std::iter::from_fn(|| app_completed.try_recv().ok())
    .find(|signal| {
      signal
        .metadata
        .get("route")
        .is_some_and(|route| route == "/users/{id}")
    })
    .expect("app arbiter receives the matched route event");
  for signal in [
    completed.try_recv().unwrap(),
    route_completed.try_recv().unwrap(),
    app_signal,
  ] {
    assert_eq!(signal.metadata["route"], "/users/{id}");
    assert_eq!(signal.metadata["status"], "200");
    assert!(signal.metadata["duration_us"].parse::<u64>().is_ok());
  }
}
