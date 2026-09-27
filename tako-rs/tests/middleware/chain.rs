use super::*;

#[tokio::test]
async fn middleware_chain_order() {
  let mut router = Router::new();
  router.route(Method::GET, "/", |_req: Request| async { "handler" });

  router.middleware(|req: Request, next: tako::middleware::Next| async move {
    let mut resp = next.run(req).await;
    resp
      .headers_mut()
      .insert("x-order", "first".parse().unwrap());
    resp
  });
  router.middleware(|req: Request, next: tako::middleware::Next| async move {
    let mut resp = next.run(req).await;
    resp
      .headers_mut()
      .insert("x-order", "second".parse().unwrap());
    resp
  });

  let resp = router.dispatch(make_req(Method::GET, "/")).await;
  assert_eq!(resp.headers().get("x-order").unwrap(), "first");
}

#[cfg(not(feature = "compio"))]
#[tokio::test]
async fn timeout_returns_503_when_exceeded() {
  use std::time::Duration;

  use tako::middleware::timeout::Timeout;

  let mut router = Router::new();
  router.route(Method::GET, "/slow", |_req: Request| async {
    tokio::time::sleep(Duration::from_millis(50)).await;
    "done"
  });
  router.middleware(Timeout::new(Duration::from_millis(5)).into_middleware());

  let resp = router.dispatch(make_req(Method::GET, "/slow")).await;
  assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[cfg(not(feature = "compio"))]
#[tokio::test]
async fn timeout_passes_when_within_deadline() {
  use std::time::Duration;

  use tako::middleware::timeout::Timeout;

  let mut router = Router::new();
  router.route(Method::GET, "/fast", |_req: Request| async { "done" });
  router.middleware(Timeout::new(Duration::from_secs(1)).into_middleware());

  let resp = router.dispatch(make_req(Method::GET, "/fast")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "done");
}

#[tokio::test]
async fn traceparent_generates_when_missing() {
  use tako::middleware::traceparent::TRACEPARENT;
  use tako::middleware::traceparent::Traceparent;

  let mut router = Router::new();
  router.route(Method::GET, "/", |_req: Request| async { "ok" });
  router.middleware(Traceparent::new().into_middleware());

  let resp = router.dispatch(make_req(Method::GET, "/")).await;
  let header = resp.headers().get(TRACEPARENT).unwrap().to_str().unwrap();
  assert!(header.starts_with("00-"));
  let parts: Vec<&str> = header.split('-').collect();
  assert_eq!(parts.len(), 4);
  assert_eq!(parts[1].len(), 32);
  assert_eq!(parts[2].len(), 16);
  assert_eq!(parts[3].len(), 2);
}

#[tokio::test]
async fn traceparent_propagates_inbound() {
  use tako::middleware::traceparent::TRACEPARENT;
  use tako::middleware::traceparent::TraceContext;
  use tako::middleware::traceparent::Traceparent;

  let mut router = Router::new();
  router.route(Method::GET, "/", |req: Request| async move {
    let ctx = req.extensions().get::<TraceContext>().cloned();
    ctx.map(|c| c.trace_id).unwrap_or_default()
  });
  router.middleware(Traceparent::new().into_middleware());

  let mut req = make_req(Method::GET, "/");
  req.headers_mut().insert(
    "traceparent",
    "00-0123456789abcdef0123456789abcdef-0011223344556677-01"
      .parse()
      .unwrap(),
  );
  let resp = router.dispatch(req).await;
  let response_header = resp.headers().get(TRACEPARENT).unwrap().to_str().unwrap();
  assert!(response_header.contains("0123456789abcdef0123456789abcdef"));
  assert_eq!(body_str(resp).await, "0123456789abcdef0123456789abcdef");
}

#[tokio::test]
async fn problem_json_rewrites_text_404() {
  use tako::middleware::problem_json::ProblemJson;

  let mut router = Router::new();
  router.route(Method::GET, "/x", |_req: Request| async {
    (StatusCode::NOT_FOUND, "missing")
  });
  router.middleware(ProblemJson::new().into_middleware());

  let resp = router.dispatch(make_req(Method::GET, "/x")).await;
  assert_eq!(resp.status(), StatusCode::NOT_FOUND);
  assert_eq!(
    resp.headers().get("content-type").unwrap(),
    "application/problem+json"
  );
  let body = body_str(resp).await;
  assert!(body.contains("\"status\":404"));
}

#[tokio::test]
async fn etag_304_on_match() {
  use tako::middleware::etag::ETag;

  let mut router = Router::new();
  router.route(Method::GET, "/", |_req: Request| async { "hello" });
  router.middleware(ETag::new().into_middleware());

  let resp = router.dispatch(make_req(Method::GET, "/")).await;
  let etag = resp
    .headers()
    .get("etag")
    .unwrap()
    .to_str()
    .unwrap()
    .to_string();

  let mut req = make_req(Method::GET, "/");
  req
    .headers_mut()
    .insert("if-none-match", etag.parse().unwrap());
  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
  assert!(
    etag.starts_with("W/\""),
    "expected weak validator prefix, got: {etag}"
  );
}

#[tokio::test]
async fn etag_if_modified_since_rfc850() {
  use tako::middleware::etag::ETag;

  let mut router = Router::new();
  router.route(Method::GET, "/", |_req: Request| async {
    let mut resp = http::Response::new(TakoBody::from("hello"));
    resp
      .headers_mut()
      .insert("etag", "\"abc\"".parse().unwrap());
    resp.headers_mut().insert(
      "last-modified",
      "Sun, 06 Nov 1994 08:49:37 GMT".parse().unwrap(),
    );
    resp
  });
  router.middleware(ETag::new().into_middleware());

  let mut req = make_req(Method::GET, "/");
  req.headers_mut().insert(
    "if-modified-since",
    "Sunday, 06-Nov-94 08:49:37 GMT".parse().unwrap(),
  );
  let resp = router.dispatch(req).await;
  assert_eq!(
    resp.status(),
    StatusCode::NOT_MODIFIED,
    "RFC 850 If-Modified-Since must match IMF-fixdate Last-Modified"
  );
}

#[tokio::test]
async fn tenant_extracted_from_header() {
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

  let mut req = make_req(Method::GET, "/");
  req
    .headers_mut()
    .insert("x-tenant-id", "acme".parse().unwrap());
  let resp = router.dispatch(req).await;
  assert_eq!(body_str(resp).await, "acme");
}
