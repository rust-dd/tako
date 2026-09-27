use super::*;

#[tokio::test]
async fn csrf_safe_method_sets_cookie() {
  use tako::middleware::csrf::Csrf;

  let mut router = Router::new();
  router.route(Method::GET, "/", |_req: Request| async { "ok" });
  router.middleware(Csrf::new().into_middleware());

  let resp = router.dispatch(make_req(Method::GET, "/")).await;
  assert_eq!(resp.status(), StatusCode::OK);

  let set_cookie = resp
    .headers()
    .get_all("set-cookie")
    .iter()
    .find(|v| v.to_str().unwrap().starts_with("csrf_token="))
    .expect("csrf cookie should be set");
  let cookie_str = set_cookie.to_str().unwrap();
  assert!(cookie_str.contains("SameSite=Strict"));
}

#[tokio::test]
async fn csrf_post_without_token_rejected() {
  use tako::middleware::csrf::Csrf;

  let mut router = Router::new();
  router.route(Method::POST, "/submit", |_req: Request| async { "ok" });
  router.middleware(Csrf::new().bind_to_session(false).into_middleware());

  let resp = router.dispatch(make_req(Method::POST, "/submit")).await;
  assert_eq!(resp.status(), StatusCode::FORBIDDEN);
  assert_eq!(body_str(resp).await, "CSRF token mismatch");
}

#[tokio::test]
async fn csrf_post_with_matching_token() {
  use tako::middleware::csrf::Csrf;

  let mut router = Router::new();
  router.route(Method::POST, "/submit", |_req: Request| async { "ok" });
  router.middleware(Csrf::new().bind_to_session(false).into_middleware());

  let token = "test-csrf-token-12345";
  let mut req = make_req(Method::POST, "/submit");
  req
    .headers_mut()
    .insert("cookie", format!("csrf_token={token}").parse().unwrap());
  req
    .headers_mut()
    .insert("x-csrf-token", token.parse().unwrap());

  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn csrf_post_mismatched_tokens() {
  use tako::middleware::csrf::Csrf;

  let mut router = Router::new();
  router.route(Method::POST, "/submit", |_req: Request| async { "ok" });
  router.middleware(Csrf::new().bind_to_session(false).into_middleware());

  let mut req = make_req(Method::POST, "/submit");
  req
    .headers_mut()
    .insert("cookie", "csrf_token=token_a".parse().unwrap());
  req
    .headers_mut()
    .insert("x-csrf-token", "token_b".parse().unwrap());

  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn csrf_bind_to_session_without_session_rejects_post() {
  use tako::middleware::csrf::Csrf;

  let mut router = Router::new();
  router.route(Method::POST, "/submit", |_req: Request| async { "ok" });
  router.middleware(Csrf::new().into_middleware());

  let token = "test-csrf-token-12345";
  let mut req = make_req(Method::POST, "/submit");
  req
    .headers_mut()
    .insert("cookie", format!("csrf_token={token}").parse().unwrap());
  req
    .headers_mut()
    .insert("x-csrf-token", token.parse().unwrap());

  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), StatusCode::FORBIDDEN);
  assert_eq!(
    body_str(resp).await,
    "CSRF: session required for token binding"
  );
}

#[tokio::test]
async fn csrf_exempt_path() {
  use tako::middleware::csrf::Csrf;

  let mut router = Router::new();
  router.route(Method::POST, "/api/webhook", |_req: Request| async { "ok" });
  router.middleware(Csrf::new().exempt("/api/webhook").into_middleware());

  let resp = router
    .dispatch(make_req(Method::POST, "/api/webhook"))
    .await;
  assert_eq!(resp.status(), StatusCode::OK);
}
