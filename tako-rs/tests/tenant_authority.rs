use http::StatusCode;
use http::Version;
use http_body_util::BodyExt;
use tako::body::TakoBody;
use tako::conn_info::ConnInfo;
use tako::extractors::FromRequest;
use tako::extractors::uri_parts::Host;
use tako::extractors::uri_parts::UriPartsConfig;
use tako::middleware::IntoMiddleware;
use tako::middleware::tenant::Tenant;
use tako::middleware::tenant::TenantMiddleware;
use tako::router::Router;
use tako::types::Request;

fn router() -> Router {
  let mut router = Router::new();
  router.middleware(
    TenantMiddleware::from_subdomain()
      .require(true)
      .into_middleware(),
  );
  router.get("/", |request: Request| async move {
    request.extensions().get::<Tenant>().unwrap().0.clone()
  });
  router
}

fn request(version: Version, uri: &str, host: Option<&str>) -> Request {
  let mut request = http::Request::builder().version(version).uri(uri);
  if let Some(host) = host {
    request = request.header("host", host);
  }
  request.body(TakoBody::empty()).unwrap()
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn tenant_resolves_host_and_authority_consistently() {
  let router = router();
  for (version, uri, host) in [
    (Version::HTTP_11, "/", Some("Acme.example.test:8080")),
    (Version::HTTP_2, "https://Acme.example.test:8443/", None),
    (Version::HTTP_3, "https://acme.example.test/", None),
    (
      Version::HTTP_2,
      "https://acme.example.test/",
      Some("ACME.example.test:443"),
    ),
  ] {
    let response = router.dispatch(request(version, uri, host)).await;
    assert_eq!(response.status(), StatusCode::OK, "{version:?} {uri}");
    assert_eq!(
      response.into_body().collect().await.unwrap().to_bytes(),
      "acme"
    );
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn conflicting_hosts_are_not_tenant_identifiers() {
  let router = router();
  for host in ["other.example.test", "acme.example.test:8443"] {
    let mut req = request(Version::HTTP_2, "https://acme.example.test/", Some(host));
    assert!(Host::from_request(&mut req).await.is_err());
    assert_eq!(router.dispatch(req).await.status(), StatusCode::BAD_REQUEST);
  }
  for host in [
    "127.0.0.1:8080",
    "[::1]:8080",
    "localhost",
    "user@acme.example.test",
    "acme.example.test:bad",
    "acme.example.test:65536",
    "acme.example.test:",
  ] {
    assert_eq!(
      router
        .dispatch(request(Version::HTTP_11, "/", Some(host)))
        .await
        .status(),
      StatusCode::BAD_REQUEST
    );
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn multiple_hosts_are_rejected_by_request_and_parts_extractors() {
  use tako::extractors::FromRequestParts;

  let mut req = request(Version::HTTP_11, "/", Some("acme.example.test"));
  req
    .headers_mut()
    .append("host", "other.example.test".parse().unwrap());
  assert!(Host::from_request(&mut req).await.is_err());
  let (mut parts, _) = req.into_parts();
  assert!(Host::from_request_parts(&mut parts).await.is_err());
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn tenant_forwarding_requires_a_trusted_peer() {
  let trusted: std::net::SocketAddr = "10.0.0.5:12345".parse().unwrap();
  let mut router = router();
  router.with_state(UriPartsConfig::default().with_trusted_proxy(trusted.ip()));
  for (peer, expected) in [
    (trusted, "acme"),
    ("203.0.113.5:12345".parse().unwrap(), "backend"),
  ] {
    let mut req = request(Version::HTTP_3, "https://backend.internal/", None);
    req.extensions_mut().insert(ConnInfo::tcp(peer));
    req
      .headers_mut()
      .insert("x-forwarded-host", "Acme.example.test:443".parse().unwrap());
    let response = router.dispatch(req).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
      response.into_body().collect().await.unwrap().to_bytes(),
      expected
    );
  }
  let mut req = request(
    Version::HTTP_2,
    "https://backend.internal/",
    Some("other.internal"),
  );
  req.extensions_mut().insert(ConnInfo::tcp(trusted));
  req
    .headers_mut()
    .insert("forwarded", "host=acme.example.test".parse().unwrap());
  assert_eq!(router.dispatch(req).await.status(), StatusCode::BAD_REQUEST);
  let mut req = request(Version::HTTP_2, "https://backend.internal/", None);
  req.extensions_mut().insert(ConnInfo::tcp(trusted));
  req.headers_mut().insert(
    "forwarded",
    "for=192.0.2.1; Host=\"Acme.example.test:443\", host=ignored.test"
      .parse()
      .unwrap(),
  );
  let response = router.dispatch(req).await;
  assert_eq!(response.status(), StatusCode::OK);
  assert_eq!(
    response.into_body().collect().await.unwrap().to_bytes(),
    "acme"
  );
}
