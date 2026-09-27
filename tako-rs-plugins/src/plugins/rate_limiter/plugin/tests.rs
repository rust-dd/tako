use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use tako_rs_core::body::TakoBody;
use tako_rs_core::router::Router;

use super::Algorithm;
use super::BucketKey;
use super::RateLimiterBuilder;
use super::evict_stale;
use super::idle_retention;

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn janitor_handles_both_algorithms_and_preserves_active_keys() {
  for algorithm in [Algorithm::TokenBucket, Algorithm::Gcra] {
    let plugin = RateLimiterBuilder::new()
      .algorithm(algorithm)
      .key_fn(|request| Some(request.uri().path().to_owned()))
      .build();
    let mut router = Router::new();
    router.get("/{key}", || async { "ok" });
    router.plugin(plugin.clone());
    router.setup_plugins_once().unwrap();
    assert!(plugin.task_started.load(Ordering::SeqCst));
    for key in ["stale", "active"] {
      let request = http::Request::builder()
        .uri(format!("/{key}"))
        .body(TakoBody::empty())
        .unwrap();
      router.dispatch(request).await;
    }
    plugin
      .store
      .get_sync(&BucketKey::Custom("/stale".into()))
      .unwrap()
      .get()
      .lock()
      .last_refill = Instant::now()
      .checked_sub(Duration::from_secs(301))
      .unwrap();
    evict_stale(&plugin.store, idle_retention(&plugin.cfg)).await;
    assert!(
      !plugin
        .store
        .contains_sync(&BucketKey::Custom("/stale".into()))
    );
    assert!(
      plugin
        .store
        .contains_sync(&BucketKey::Custom("/active".into()))
    );
  }
}

#[test]
fn idle_retention_never_resets_a_slow_quota_early() {
  let plugin = RateLimiterBuilder::new()
    .max_requests(10)
    .refill_rate(1)
    .refill_interval_ms(60_000)
    .build();
  assert_eq!(idle_retention(&plugin.cfg), Duration::from_secs(600));
  assert_eq!(
    idle_retention(&RateLimiterBuilder::new().build().cfg),
    Duration::from_secs(300)
  );
}
