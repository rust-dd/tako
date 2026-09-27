use http_body_util::BodyExt;
use tako::body::TakoBody;
use tako::extractors::ipaddr::IpAddr;
use tako::extractors::ipaddr::IpAddrConfig;
use tako::router::Router;
use tako::types::Request;

fn request(forwarded: &str, value: &str, peer: &str) -> Request {
  let mut req = http::Request::builder()
    .header(forwarded, value)
    .body(TakoBody::empty())
    .unwrap();
  req
    .extensions_mut()
    .insert(peer.parse::<std::net::SocketAddr>().unwrap());
  req
}

#[tokio::test]
async fn cidr_trust_is_router_local_and_walks_from_the_nearest_hop() {
  let mut trusted = Router::new();
  trusted.with_state(IpAddrConfig::new().trust_network("10.0.0.0/8".parse().unwrap()));
  trusted.get("/", |ip: IpAddr| async move { ip.to_string() });
  let mut untrusted = Router::new();
  untrusted.get("/", |ip: IpAddr| async move { ip.to_string() });
  for (router, expected) in [(&trusted, "192.0.2.42"), (&untrusted, "10.1.2.3")] {
    let req = request(
      "x-forwarded-for",
      "203.0.113.9, 192.0.2.42, 10.2.3.4",
      "10.1.2.3:80",
    );
    let bytes = router
      .dispatch(req)
      .await
      .into_body()
      .collect()
      .await
      .unwrap()
      .to_bytes();
    assert_eq!(bytes, expected);
  }
}

#[test]
fn forwarded_parameters_support_ipv6_and_stop_at_opaque_hops() {
  let resolve = |value: &str| {
    let mut req = request("forwarded", value, "127.0.0.1:80");
    req
      .extensions_mut()
      .insert(IpAddrConfig::new().trust("127.0.0.1".parse().unwrap()));
    IpAddr::resolve(req.extensions(), req.headers())
      .unwrap()
      .to_string()
  };
  assert_eq!(
    resolve("for=\"[2001:db8::1]:443\";proto=https;by=127.0.0.1"),
    "2001:db8::1"
  );
  assert_eq!(resolve("for=203.0.113.1, for=unknown"), "127.0.0.1");
  assert_eq!(resolve("for=203.0.113.1;for=192.0.2.1"), "127.0.0.1");
  assert_eq!(resolve("for=\"203.0.113.1"), "127.0.0.1");
}

#[test]
fn repeated_forwarding_headers_preserve_hop_order_and_reject_fallback_spoofing() {
  let mut req = request("forwarded", "for=203.0.113.9", "127.0.0.1:80");
  req
    .extensions_mut()
    .insert(IpAddrConfig::new().trust("127.0.0.1".parse().unwrap()));
  req
    .headers_mut()
    .append("forwarded", "for=192.0.2.42".parse().unwrap());
  assert_eq!(
    IpAddr::resolve(req.extensions(), req.headers())
      .unwrap()
      .to_string(),
    "192.0.2.42"
  );
  req
    .headers_mut()
    .append("forwarded", "for=unknown".parse().unwrap());
  req
    .headers_mut()
    .insert("x-forwarded-for", "203.0.113.1".parse().unwrap());
  assert_eq!(
    IpAddr::resolve(req.extensions(), req.headers())
      .unwrap()
      .to_string(),
    "127.0.0.1"
  );
}
