use super::*;

#[tokio::test]
async fn api_key_valid() {
  use tako::middleware::api_key_auth::ApiKeyAuth;

  let mut router = Router::new();
  router.route(Method::GET, "/api", |_req: Request| async { "ok" });
  router.middleware(ApiKeyAuth::new("secret-key").into_middleware());

  let mut req = make_req(Method::GET, "/api");
  req
    .headers_mut()
    .insert("x-api-key", "secret-key".parse().unwrap());

  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "ok");
}

#[tokio::test]
async fn api_key_missing() {
  use tako::middleware::api_key_auth::ApiKeyAuth;

  let mut router = Router::new();
  router.route(Method::GET, "/api", |_req: Request| async { "ok" });
  router.middleware(ApiKeyAuth::new("secret-key").into_middleware());

  let resp = router.dispatch(make_req(Method::GET, "/api")).await;
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
  assert_eq!(resp.headers().get("www-authenticate").unwrap(), "ApiKey");
  assert_eq!(body_str(resp).await, "API key is missing");
}

#[tokio::test]
async fn api_key_invalid() {
  use tako::middleware::api_key_auth::ApiKeyAuth;

  let mut router = Router::new();
  router.route(Method::GET, "/api", |_req: Request| async { "ok" });
  router.middleware(ApiKeyAuth::new("secret-key").into_middleware());

  let mut req = make_req(Method::GET, "/api");
  req
    .headers_mut()
    .insert("x-api-key", "wrong-key".parse().unwrap());

  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
  assert_eq!(body_str(resp).await, "Invalid API key");
}

#[tokio::test]
async fn api_key_from_query() {
  use tako::middleware::api_key_auth::ApiKeyAuth;
  use tako::middleware::api_key_auth::ApiKeyLocation;

  let mut router = Router::new();
  router.route(Method::GET, "/api", |_req: Request| async { "ok" });
  router.middleware(
    ApiKeyAuth::new("qkey")
      .location(ApiKeyLocation::Query("api_key"))
      .into_middleware(),
  );

  let resp = router
    .dispatch(make_req(Method::GET, "/api?api_key=qkey"))
    .await;
  assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn api_key_with_verify() {
  use tako::middleware::api_key_auth::ApiKeyAuth;

  let mut router = Router::new();
  router.route(Method::GET, "/api", |_req: Request| async { "ok" });
  router.middleware(ApiKeyAuth::with_verify(|key| key.starts_with("valid_")).into_middleware());

  let mut req = make_req(Method::GET, "/api");
  req
    .headers_mut()
    .insert("x-api-key", "valid_abc".parse().unwrap());
  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), StatusCode::OK);

  let mut req2 = make_req(Method::GET, "/api");
  req2
    .headers_mut()
    .insert("x-api-key", "invalid".parse().unwrap());
  let resp2 = router.dispatch(req2).await;
  assert_eq!(resp2.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn basic_auth_valid() {
  use tako::middleware::basic_auth::BasicAuth;

  let mut router = Router::new();
  router.route(Method::GET, "/secure", |_req: Request| async { "ok" });
  router.middleware(BasicAuth::single("admin", "password").into_middleware());

  let encoded =
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, "admin:password");
  let mut req = make_req(Method::GET, "/secure");
  req
    .headers_mut()
    .insert("authorization", format!("Basic {encoded}").parse().unwrap());

  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn basic_auth_missing() {
  use tako::middleware::basic_auth::BasicAuth;

  let mut router = Router::new();
  router.route(Method::GET, "/secure", |_req: Request| async { "ok" });
  router.middleware(BasicAuth::single("admin", "password").into_middleware());

  let resp = router.dispatch(make_req(Method::GET, "/secure")).await;
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
  assert!(
    resp
      .headers()
      .get("www-authenticate")
      .unwrap()
      .to_str()
      .unwrap()
      .contains("Basic realm=")
  );
  assert_eq!(body_str(resp).await, "Missing credentials");
}

#[tokio::test]
async fn basic_auth_wrong_password() {
  use tako::middleware::basic_auth::BasicAuth;

  let mut router = Router::new();
  router.route(Method::GET, "/secure", |_req: Request| async { "ok" });
  router.middleware(BasicAuth::single("admin", "password").into_middleware());

  let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, "admin:wrong");
  let mut req = make_req(Method::GET, "/secure");
  req
    .headers_mut()
    .insert("authorization", format!("Basic {encoded}").parse().unwrap());

  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn basic_auth_custom_realm() {
  use tako::middleware::basic_auth::BasicAuth;

  let mut router = Router::new();
  router.route(Method::GET, "/admin", |_req: Request| async { "ok" });
  router.middleware(
    BasicAuth::single("admin", "pass")
      .realm("Admin Area")
      .into_middleware(),
  );

  let resp = router.dispatch(make_req(Method::GET, "/admin")).await;
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
  let www_auth = resp
    .headers()
    .get("www-authenticate")
    .unwrap()
    .to_str()
    .unwrap()
    .to_string();
  assert!(www_auth.contains("Admin Area"));
}

#[tokio::test]
async fn bearer_auth_valid() {
  use tako::middleware::bearer_auth::BearerAuth;

  let mut router = Router::new();
  router.route(Method::GET, "/api", |_req: Request| async { "ok" });
  router.middleware(BearerAuth::static_token("my-token").into_middleware());

  let mut req = make_req(Method::GET, "/api");
  req
    .headers_mut()
    .insert("authorization", "Bearer my-token".parse().unwrap());

  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn bearer_auth_missing() {
  use tako::middleware::bearer_auth::BearerAuth;

  let mut router = Router::new();
  router.route(Method::GET, "/api", |_req: Request| async { "ok" });
  router.middleware(BearerAuth::static_token("my-token").into_middleware());

  let resp = router.dispatch(make_req(Method::GET, "/api")).await;
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
  assert_eq!(body_str(resp).await, "Token is missing");
}

#[tokio::test]
async fn bearer_auth_wrong_token() {
  use tako::middleware::bearer_auth::BearerAuth;

  let mut router = Router::new();
  router.route(Method::GET, "/api", |_req: Request| async { "ok" });
  router.middleware(BearerAuth::static_token("my-token").into_middleware());

  let mut req = make_req(Method::GET, "/api");
  req
    .headers_mut()
    .insert("authorization", "Bearer wrong".parse().unwrap());

  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn bearer_auth_wrong_scheme() {
  use tako::middleware::bearer_auth::BearerAuth;

  let mut router = Router::new();
  router.route(Method::GET, "/api", |_req: Request| async { "ok" });
  router.middleware(BearerAuth::static_token("my-token").into_middleware());

  let mut req = make_req(Method::GET, "/api");
  req
    .headers_mut()
    .insert("authorization", "Basic dXNlcjpwYXNz".parse().unwrap());

  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn bearer_auth_scheme_is_case_insensitive() {
  use tako::middleware::bearer_auth::BearerAuth;

  let mut router = Router::new();
  router.route(Method::GET, "/api", |_req: Request| async { "ok" });
  router.middleware(BearerAuth::static_token("my-token").into_middleware());

  for variant in &[
    "Bearer my-token",
    "bearer my-token",
    "BEARER my-token",
    "BeArEr my-token",
  ] {
    let mut req = make_req(Method::GET, "/api");
    req
      .headers_mut()
      .insert("authorization", variant.parse().unwrap());
    let resp = router.dispatch(req).await;
    assert_eq!(
      resp.status(),
      StatusCode::OK,
      "scheme {variant:?} must be accepted (RFC 7235 case-insensitive)"
    );
  }
}

#[tokio::test]
async fn basic_auth_scheme_is_case_insensitive() {
  use tako::middleware::basic_auth::BasicAuth;

  let mut router = Router::new();
  router.route(Method::GET, "/api", |_req: Request| async { "ok" });
  router.middleware(BasicAuth::single("user", "pass").into_middleware());

  for variant in &[
    "Basic dXNlcjpwYXNz",
    "basic dXNlcjpwYXNz",
    "BASIC dXNlcjpwYXNz",
  ] {
    let mut req = make_req(Method::GET, "/api");
    req
      .headers_mut()
      .insert("authorization", variant.parse().unwrap());
    let resp = router.dispatch(req).await;
    assert_eq!(
      resp.status(),
      StatusCode::OK,
      "scheme {variant:?} must be accepted (RFC 7235 case-insensitive)"
    );
  }
}
