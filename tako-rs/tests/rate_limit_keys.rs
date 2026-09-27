#![cfg(feature = "plugins")]

use std::net::SocketAddr;

use http::StatusCode;
use tako::body::TakoBody;
use tako::extractors::ipaddr::IpAddrConfig;
use tako::plugins::rate_limiter::RateLimiterBuilder;
use tako::router::Router;
use tako::types::Request;

fn request(peer: &str, forwarded: &str) -> Request {
  let mut request = http::Request::builder()
    .header("x-forwarded-for", forwarded)
    .body(TakoBody::empty())
    .unwrap();
  request
    .extensions_mut()
    .insert(peer.parse::<SocketAddr>().unwrap());
  request
}

fn router(builder: RateLimiterBuilder) -> Router {
  let mut router = Router::new();
  router.get("/", || async { "ok" });
  router.plugin(
    builder
      .max_requests(1)
      .refill_rate(1)
      .refill_interval_ms(60_000)
      .build(),
  );
  router.setup_plugins_once().unwrap();
  router
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn client_ip_mode_only_trusts_configured_proxies() {
  let mut trusted = router(RateLimiterBuilder::new().client_ip(true));
  trusted.with_state(IpAddrConfig::new().trust_network("10.0.0.0/8".parse().unwrap()));
  for client in ["192.0.2.1", "192.0.2.2"] {
    assert_eq!(
      trusted
        .dispatch(request("10.0.0.1:5000", client))
        .await
        .status(),
      StatusCode::OK
    );
  }
  assert_eq!(
    trusted
      .dispatch(request("10.0.0.1:5001", "192.0.2.1"))
      .await
      .status(),
    StatusCode::TOO_MANY_REQUESTS
  );
  let untrusted = router(RateLimiterBuilder::new().client_ip(true));
  assert_eq!(
    untrusted
      .dispatch(request("10.0.0.1:5000", "192.0.2.1"))
      .await
      .status(),
    StatusCode::OK
  );
  assert_eq!(
    untrusted
      .dispatch(request("10.0.0.1:5000", "192.0.2.2"))
      .await
      .status(),
    StatusCode::TOO_MANY_REQUESTS
  );
  let mut peer_only = router(RateLimiterBuilder::new());
  peer_only.with_state(IpAddrConfig::new().trust_network("10.0.0.0/8".parse().unwrap()));
  peer_only
    .dispatch(request("10.0.0.1:5000", "192.0.2.1"))
    .await;
  assert_eq!(
    peer_only
      .dispatch(request("10.0.0.1:5000", "192.0.2.2"))
      .await
      .status(),
    StatusCode::TOO_MANY_REQUESTS
  );
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn ipv6_prefixes_aggregate_without_merging_distinct_subnets() {
  let aggregated = router(RateLimiterBuilder::new().ipv6_prefix(64));
  assert_eq!(
    aggregated
      .dispatch(request("[2001:db8:1:2::1]:1", ""))
      .await
      .status(),
    StatusCode::OK
  );
  let rejected = aggregated
    .dispatch(request("[2001:db8:1:2::ffff]:2", ""))
    .await;
  assert_eq!(rejected.status(), StatusCode::TOO_MANY_REQUESTS);
  assert_eq!(rejected.headers()["ratelimit-limit"], "1");
  assert_eq!(rejected.headers()["ratelimit-remaining"], "0");
  assert!(rejected.headers().contains_key("retry-after"));
  assert_eq!(
    aggregated
      .dispatch(request("[2001:db8:1:3::1]:3", ""))
      .await
      .status(),
    StatusCode::OK
  );
  let individual = router(RateLimiterBuilder::new());
  for peer in ["[2001:db8::1]:1", "[2001:db8::2]:2"] {
    assert_eq!(
      individual.dispatch(request(peer, "")).await.status(),
      StatusCode::OK
    );
  }
  let mapped = router(RateLimiterBuilder::new().ipv6_prefix(64));
  mapped.dispatch(request("192.0.2.1:1", "")).await;
  assert_eq!(
    mapped
      .dispatch(request("[::ffff:192.0.2.1]:2", ""))
      .await
      .status(),
    StatusCode::TOO_MANY_REQUESTS
  );
}
