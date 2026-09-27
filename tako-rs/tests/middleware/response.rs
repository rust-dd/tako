use super::*;

#[tokio::test]
async fn body_limit_within_limit() {
  use tako::middleware::body_limit::BodyLimit;

  let mut router = Router::new();
  router.route(Method::POST, "/upload", |_req: Request| async { "ok" });
  router.middleware(BodyLimit::new_with_dynamic(1024, |_| 1024).into_middleware());

  let req = make_req_with_body(Method::POST, "/upload", "small body");
  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn body_limit_content_length_reject() {
  use tako::middleware::body_limit::BodyLimit;

  let mut router = Router::new();
  router.route(Method::POST, "/upload", |_req: Request| async { "ok" });
  router.middleware(BodyLimit::new_with_dynamic(10, |_| 10).into_middleware());

  let mut req = make_req_with_body(Method::POST, "/upload", "this body is too large");
  req
    .headers_mut()
    .insert("content-length", "22".parse().unwrap());

  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn body_limit_runtime_reject() {
  use tako::middleware::body_limit::BodyLimit;

  let mut router = Router::new();
  router.route(Method::POST, "/upload", |req: Request| async move {
    let (_, body) = req.into_parts();
    match body.collect().await {
      Ok(_) => http::Response::builder()
        .status(StatusCode::OK)
        .body(TakoBody::from("ok"))
        .unwrap(),
      Err(_) => http::Response::builder()
        .status(StatusCode::PAYLOAD_TOO_LARGE)
        .body(TakoBody::from("Body exceeds allowed size"))
        .unwrap(),
    }
  });
  router.middleware(BodyLimit::new_with_dynamic(5, |_| 5).into_middleware());

  let req = make_req_with_body(Method::POST, "/upload", "too large");
  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
  assert_eq!(body_str(resp).await, "Body exceeds allowed size");
}

#[tokio::test]
async fn security_headers_default() {
  use tako::middleware::security_headers::SecurityHeaders;

  let mut router = Router::new();
  router.route(Method::GET, "/", |_req: Request| async { "ok" });
  router.middleware(SecurityHeaders::new().into_middleware());

  let resp = router.dispatch(make_req(Method::GET, "/")).await;
  assert_eq!(
    resp.headers().get("x-content-type-options").unwrap(),
    "nosniff"
  );
  assert_eq!(resp.headers().get("x-frame-options").unwrap(), "DENY");
  assert!(resp.headers().get("x-xss-protection").is_none());
  assert_eq!(
    resp.headers().get("referrer-policy").unwrap(),
    "strict-origin-when-cross-origin"
  );
  assert!(resp.headers().get("strict-transport-security").is_none());
}

#[tokio::test]
async fn security_headers_with_hsts() {
  use tako::middleware::security_headers::SecurityHeaders;

  let mut router = Router::new();
  router.route(Method::GET, "/", |_req: Request| async { "ok" });
  router.middleware(
    SecurityHeaders::new()
      .hsts(true)
      .hsts_max_age(300)
      .into_middleware(),
  );

  let resp = router.dispatch(make_req(Method::GET, "/")).await;
  let hsts = resp
    .headers()
    .get("strict-transport-security")
    .unwrap()
    .to_str()
    .unwrap();
  assert!(hsts.contains("max-age=300"));
  assert!(hsts.contains("includeSubDomains"));
}

#[tokio::test]
async fn security_headers_custom_frame_options() {
  use tako::middleware::security_headers::SecurityHeaders;

  let mut router = Router::new();
  router.route(Method::GET, "/", |_req: Request| async { "ok" });
  router.middleware(
    SecurityHeaders::new()
      .frame_options("SAMEORIGIN")
      .into_middleware(),
  );

  let resp = router.dispatch(make_req(Method::GET, "/")).await;
  assert_eq!(resp.headers().get("x-frame-options").unwrap(), "SAMEORIGIN");
}

#[tokio::test]
async fn request_id_generated() {
  use tako::middleware::request_id::RequestId;

  let mut router = Router::new();
  router.route(Method::GET, "/", |_req: Request| async { "ok" });
  router.middleware(RequestId::new().into_middleware());

  let resp = router.dispatch(make_req(Method::GET, "/")).await;
  assert!(resp.headers().get("x-request-id").is_some());
}

#[tokio::test]
async fn request_id_preserved() {
  use tako::middleware::request_id::RequestId;

  let mut router = Router::new();
  router.route(Method::GET, "/", |_req: Request| async { "ok" });
  router.middleware(RequestId::new().into_middleware());

  let mut req = make_req(Method::GET, "/");
  req
    .headers_mut()
    .insert("x-request-id", "abc123".parse().unwrap());

  let resp = router.dispatch(req).await;
  assert_eq!(resp.headers().get("x-request-id").unwrap(), "abc123");
}

#[tokio::test]
async fn request_id_custom_generator() {
  use tako::middleware::request_id::RequestId;

  let mut router = Router::new();
  router.route(Method::GET, "/", |_req: Request| async { "ok" });
  router.middleware(
    RequestId::new()
      .generator(|| "fixed-id".to_string())
      .into_middleware(),
  );

  let resp = router.dispatch(make_req(Method::GET, "/")).await;
  assert_eq!(resp.headers().get("x-request-id").unwrap(), "fixed-id");
}

#[tokio::test]
async fn request_id_custom_header() {
  use tako::middleware::request_id::RequestId;

  let mut router = Router::new();
  router.route(Method::GET, "/", |_req: Request| async { "ok" });
  router.middleware(
    RequestId::new()
      .header_name("X-Correlation-ID")
      .generator(|| "corr-123".to_string())
      .into_middleware(),
  );

  let resp = router.dispatch(make_req(Method::GET, "/")).await;
  assert_eq!(resp.headers().get("x-correlation-id").unwrap(), "corr-123");
}
