use http::Method;
use http::StatusCode;
use http_body_util::BodyExt;
use tako::body::TakoBody;
use tako::middleware::IntoMiddleware;
use tako::router::Router;
use tako::types::Request;

fn make_req(method: Method, uri: &str) -> Request {
  http::Request::builder()
    .method(method)
    .uri(uri)
    .body(TakoBody::empty())
    .unwrap()
}

fn make_req_with_body(method: Method, uri: &str, body: &str) -> Request {
  http::Request::builder()
    .method(method)
    .uri(uri)
    .body(TakoBody::from(body.to_string()))
    .unwrap()
}

async fn body_str(resp: tako::types::Response) -> String {
  let bytes = resp.into_body().collect().await.unwrap().to_bytes();
  String::from_utf8(bytes.to_vec()).unwrap()
}

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
  assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
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
  assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

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
  router.route(Method::POST, "/upload", |_req: Request| async { "ok" });
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
  assert_eq!(resp.headers().get("x-xss-protection").unwrap(), "0");
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
      .header_name("x-correlation-id")
      .generator(|| "corr-123".to_string())
      .into_middleware(),
  );

  let resp = router.dispatch(make_req(Method::GET, "/")).await;
  assert_eq!(resp.headers().get("x-correlation-id").unwrap(), "corr-123");
}

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
  router.middleware(Csrf::new().into_middleware());

  let resp = router.dispatch(make_req(Method::POST, "/submit")).await;
  assert_eq!(resp.status(), StatusCode::FORBIDDEN);
  assert_eq!(body_str(resp).await, "CSRF token mismatch");
}

#[tokio::test]
async fn csrf_post_with_matching_token() {
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
  assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn csrf_post_mismatched_tokens() {
  use tako::middleware::csrf::Csrf;

  let mut router = Router::new();
  router.route(Method::POST, "/submit", |_req: Request| async { "ok" });
  router.middleware(Csrf::new().into_middleware());

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

#[tokio::test]
async fn session_new_request_sets_cookie() {
  use tako::middleware::session::SessionMiddleware;

  let mut router = Router::new();
  router.route(Method::GET, "/", |_req: Request| async { "ok" });
  router.middleware(SessionMiddleware::new().into_middleware());

  let resp = router.dispatch(make_req(Method::GET, "/")).await;
  assert_eq!(resp.status(), StatusCode::OK);

  let set_cookie = resp
    .headers()
    .get_all("set-cookie")
    .iter()
    .find(|v| v.to_str().unwrap().starts_with("tako_session="));
  assert!(set_cookie.is_some(), "session cookie should be set");
}

#[tokio::test]
async fn session_get_set_data() {
  use tako::middleware::session::Session;
  use tako::middleware::session::SessionMiddleware;

  let mut router = Router::new();
  router.route(Method::POST, "/set", |req: Request| async move {
    let session = req.extensions().get::<Session>().unwrap();
    session.set("counter", 42u32);
    "stored"
  });
  router.route(Method::GET, "/get", |req: Request| async move {
    let session = req.extensions().get::<Session>().unwrap();
    let val: Option<u32> = session.get("counter");
    format!("counter={}", val.unwrap_or(0))
  });
  router.middleware(SessionMiddleware::new().into_middleware());

  // First request: set data
  let resp = router.dispatch(make_req(Method::POST, "/set")).await;
  assert_eq!(resp.status(), StatusCode::OK);

  // Extract session cookie
  let cookie = resp
    .headers()
    .get_all("set-cookie")
    .iter()
    .find(|v| v.to_str().unwrap().starts_with("tako_session="))
    .unwrap()
    .to_str()
    .unwrap()
    .to_string();
  let session_id = cookie.split('=').nth(1).unwrap().split(';').next().unwrap();

  // Second request: get data with same session
  let mut req = make_req(Method::GET, "/get");
  req.headers_mut().insert(
    "cookie",
    format!("tako_session={session_id}").parse().unwrap(),
  );

  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "counter=42");
}

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
    // This middleware runs second, so it sets the header last (wins)
    resp
      .headers_mut()
      .insert("x-order", "second".parse().unwrap());
    resp
  });

  let resp = router.dispatch(make_req(Method::GET, "/")).await;
  // The outermost middleware (first added) runs first and sets the header,
  // but the inner middleware (second added) also sets it. Since second runs
  // after first in the response path (unwinding), the second middleware
  // actually sets the value first, then first overwrites it.
  // Actually: first mw calls next → second mw calls next → handler → second mw
  // modifies resp → first mw modifies resp. So first wins (runs last on response).
  assert_eq!(resp.headers().get("x-order").unwrap(), "first");
}
