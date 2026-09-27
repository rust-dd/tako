#![cfg(feature = "metrics-prometheus")]

use http_body_util::BodyExt;
use tako::body::TakoBody;
use tako::plugins::metrics::PrometheusMetricsConfig;
use tako::router::Router;

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn metrics_record_every_request_with_route_and_latency_and_isolate_routers() {
  let mut router = Router::new();
  let mut other = Router::new();
  PrometheusMetricsConfig::default().install(&mut router);
  PrometheusMetricsConfig::default().install(&mut other);
  other.setup_plugins_once().unwrap();
  router.get("/users/{id}", || async { "ok" });
  let mut root = Router::new();
  root.nest("", router);
  let router = root;
  for id in 0..300 {
    let req = http::Request::builder()
      .uri(format!("/users/{id}"))
      .body(TakoBody::empty())
      .unwrap();
    router.dispatch(req).await;
  }
  let scrape = |router: Router| async move {
    let req = http::Request::builder()
      .uri("/metrics")
      .body(TakoBody::empty())
      .unwrap();
    let body = router
      .dispatch(req)
      .await
      .into_body()
      .collect()
      .await
      .unwrap()
      .to_bytes();
    String::from_utf8(body.to_vec()).unwrap()
  };
  let body = scrape(router).await;
  for name in [
    "tako_http_requests_total",
    "tako_route_requests_total",
    "tako_http_request_duration_seconds_count",
  ] {
    let line = body
      .lines()
      .find(|line| line.starts_with(name) && line.contains("route=\"/users/{id}\""))
      .unwrap();
    assert!(line.ends_with(" 300"), "{line}");
  }
  assert!(!scrape(other).await.contains("/users/{id}"));
}
