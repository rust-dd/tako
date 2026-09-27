use super::*;

#[tokio::test]
async fn tenant_invalid_ids_rejected() {
  use tako::middleware::tenant::Tenant;
  use tako::middleware::tenant::TenantMiddleware;

  let mut router = Router::new();
  router.route(Method::GET, "/", |req: Request| async move {
    req
      .extensions()
      .get::<Tenant>()
      .map_or_else(|| "no-tenant".into(), |t| t.0.clone())
  });
  router.middleware(
    TenantMiddleware::from_header(http::HeaderName::from_static("x-tenant-id")).into_middleware(),
  );

  for malicious in &["..", "../etc/passwd", "a/b", "with space", ""] {
    let mut req = make_req(Method::GET, "/");
    if !malicious.is_empty() {
      req
        .headers_mut()
        .insert("x-tenant-id", malicious.parse().unwrap());
    }
    let resp = router.dispatch(req).await;
    assert_eq!(
      body_str(resp).await,
      "no-tenant",
      "expected rejection for malicious tenant: {malicious:?}"
    );
  }
}

#[tokio::test]
async fn healthcheck_live_endpoint() {
  use tako::middleware::healthcheck::Healthcheck;

  let mut router = Router::new();
  router.route(Method::GET, "/api/anything", |_req: Request| async {
    "user route"
  });
  router.middleware(Healthcheck::new().into_middleware());

  let resp = router.dispatch(make_req(Method::GET, "/live")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert!(body_str(resp).await.contains("alive"));
}

#[tokio::test]
async fn healthcheck_drain_blocks_ready() {
  use tako::middleware::healthcheck::Healthcheck;

  let mw = Healthcheck::new();
  let handle = mw.handle();

  let mut router = Router::new();
  router.route(Method::GET, "/api", |_req: Request| async { "ok" });
  router.middleware(mw.into_middleware());

  let resp = router.dispatch(make_req(Method::GET, "/ready")).await;
  assert_eq!(resp.status(), StatusCode::OK);

  handle.drain();
  let resp = router.dispatch(make_req(Method::GET, "/ready")).await;
  assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
  assert!(resp.headers().get("retry-after").is_some());
}

#[tokio::test]
async fn healthcheck_drain_post_requires_token() {
  use tako::middleware::healthcheck::Healthcheck;

  let mut router = Router::new();
  router.route(Method::GET, "/api", |_req: Request| async { "ok" });
  router.middleware(Healthcheck::new().into_middleware());

  let resp = router.dispatch(make_req(Method::POST, "/__drain")).await;
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

  let resp = router.dispatch(make_req(Method::GET, "/__drain")).await;
  assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn healthcheck_drain_token_required_for_state_change() {
  use tako::middleware::healthcheck::Healthcheck;

  let mut router = Router::new();
  router.route(Method::GET, "/api", |_req: Request| async { "ok" });
  router.middleware(Healthcheck::new().drain_token("s3cret").into_middleware());

  let resp = router.dispatch(make_req(Method::POST, "/__drain")).await;
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

  let mut req = make_req(Method::POST, "/__drain");
  req
    .headers_mut()
    .insert("x-drain-token", "wrong".parse().unwrap());
  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

  let mut req = make_req(Method::POST, "/__drain");
  req
    .headers_mut()
    .insert("x-drain-token", "s3cret".parse().unwrap());
  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn problem_json_passes_through_json() {
  use tako::middleware::problem_json::ProblemJson;

  let mut router = Router::new();
  router.route(Method::GET, "/x", |_req: Request| async {
    let mut resp = http::Response::builder()
      .status(StatusCode::BAD_REQUEST)
      .body(TakoBody::from(r#"{"foo":"bar"}"#))
      .unwrap();
    resp
      .headers_mut()
      .insert("content-type", "application/json".parse().unwrap());
    resp
  });
  router.middleware(ProblemJson::new().into_middleware());

  let resp = router.dispatch(make_req(Method::GET, "/x")).await;
  assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
  assert_eq!(
    resp.headers().get("content-type").unwrap(),
    "application/json"
  );
  assert_eq!(body_str(resp).await, r#"{"foo":"bar"}"#);
}
