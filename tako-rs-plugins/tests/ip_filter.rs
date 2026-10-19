#![cfg(feature = "ip-filter")]

use std::net::SocketAddr;

use http::StatusCode;
use tako_rs_core::body::TakoBody;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;
use tako_rs_plugins::middleware::ip_filter::IpFilter;

fn router(filter: IpFilter) -> Router {
  let mut router = Router::new();
  router.middleware(filter.into_middleware());
  router.get("/", || async { StatusCode::NO_CONTENT });
  router
}

fn request(ip: Option<&str>) -> Request {
  let mut request = Request::new(TakoBody::empty());
  if let Some(ip) = ip {
    request
      .extensions_mut()
      .insert(ip.parse::<SocketAddr>().unwrap());
  }
  request
}

#[tokio::test]
async fn default_filter_rejects_denied_ip_with_forbidden_status() {
  let router = router(IpFilter::default().deny("192.0.2.0/24").unwrap());

  assert_eq!(
    router
      .dispatch(request(Some("192.0.2.1:8080")))
      .await
      .status(),
    StatusCode::FORBIDDEN
  );
  assert_eq!(
    router
      .dispatch(request(Some("198.51.100.1:8080")))
      .await
      .status(),
    StatusCode::NO_CONTENT
  );
}

#[tokio::test]
async fn allow_list_rejects_missing_ip_and_unix_peers() {
  let router = router(IpFilter::new().allow("192.0.2.0/24").unwrap());
  let mut unix = request(None);
  unix.extensions_mut().insert(ConnInfo::unix(None));

  for request in [request(None), unix] {
    assert_eq!(
      router.dispatch(request).await.status(),
      StatusCode::FORBIDDEN
    );
  }
  assert_eq!(
    router
      .dispatch(request(Some("192.0.2.1:8080")))
      .await
      .status(),
    StatusCode::NO_CONTENT
  );
}

#[tokio::test]
async fn unknown_peer_policy_and_custom_status_are_respected_without_allow_list() {
  for filter in [IpFilter::new(), IpFilter::default()] {
    assert_eq!(
      router(filter).dispatch(request(None)).await.status(),
      StatusCode::NO_CONTENT
    );
  }
  let router = router(
    IpFilter::default()
      .deny_unknown(true)
      .status(StatusCode::UNAUTHORIZED),
  );
  assert_eq!(
    router.dispatch(request(None)).await.status(),
    StatusCode::UNAUTHORIZED
  );
}

#[tokio::test]
async fn denied_ip_wins_over_allow_list_for_connection_metadata() {
  let router = router(
    IpFilter::new()
      .allow("192.0.2.0/24")
      .unwrap()
      .deny("192.0.2.1")
      .unwrap(),
  );
  let mut request = request(None);
  request
    .extensions_mut()
    .insert(ConnInfo::tcp("192.0.2.1:8080".parse().unwrap()));

  assert_eq!(
    router.dispatch(request).await.status(),
    StatusCode::FORBIDDEN
  );
}
