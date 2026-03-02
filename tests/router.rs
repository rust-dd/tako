use http::{Method, StatusCode};
use http_body_util::BodyExt;
use std::time::Duration;
use tako::body::TakoBody;
use tako::router::Router;
use tako::types::Request;

fn make_req(method: Method, uri: &str) -> Request {
  http::Request::builder()
    .method(method)
    .uri(uri)
    .body(TakoBody::empty())
    .unwrap()
}

async fn body_str(resp: tako::types::Response) -> String {
  let bytes = resp.into_body().collect().await.unwrap().to_bytes();
  String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn route_match_returns_200() {
  let mut router = Router::new();
  router.route(Method::GET, "/hello", |_req: Request| async { "Hello" });

  let resp = router.dispatch(make_req(Method::GET, "/hello")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "Hello");
}

#[tokio::test]
async fn route_miss_returns_404() {
  let mut router = Router::new();
  router.route(Method::GET, "/hello", |_req: Request| async { "Hello" });

  let resp = router.dispatch(make_req(Method::GET, "/notfound")).await;
  assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn different_method_returns_404() {
  let mut router = Router::new();
  router.route(Method::GET, "/hello", |_req: Request| async { "Hello" });

  let resp = router.dispatch(make_req(Method::POST, "/hello")).await;
  assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn custom_fallback() {
  let mut router = Router::new();
  router.route(Method::GET, "/hello", |_req: Request| async { "Hello" });
  router.fallback(|_req: Request| async { (StatusCode::NOT_FOUND, "Custom 404") });

  let resp = router.dispatch(make_req(Method::GET, "/nope")).await;
  assert_eq!(resp.status(), StatusCode::NOT_FOUND);
  assert_eq!(body_str(resp).await, "Custom 404");
}

#[tokio::test]
async fn tsr_redirect() {
  let mut router = Router::new();
  router.route_with_tsr(Method::GET, "/api", |_req: Request| async { "API" });

  // Exact match
  let resp = router.dispatch(make_req(Method::GET, "/api")).await;
  assert_eq!(resp.status(), StatusCode::OK);

  // Trailing slash → 307 redirect
  let resp = router.dispatch(make_req(Method::GET, "/api/")).await;
  assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
  assert_eq!(resp.headers().get("location").unwrap(), "/api");
}

#[tokio::test]
#[should_panic(expected = "Cannot route with TSR for root path")]
async fn tsr_root_panics() {
  let mut router = Router::new();
  router.route_with_tsr(Method::GET, "/", |_req: Request| async { "root" });
}

#[tokio::test]
async fn global_middleware_runs() {
  let mut router = Router::new();
  router.route(Method::GET, "/hello", |_req: Request| async { "Hello" });
  router.middleware(|req: Request, next: tako::middleware::Next| async move {
    let mut resp = next.run(req).await;
    resp.headers_mut().insert("x-middleware", "applied".parse().unwrap());
    resp
  });

  let resp = router.dispatch(make_req(Method::GET, "/hello")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(resp.headers().get("x-middleware").unwrap(), "applied");
}

#[tokio::test]
async fn router_timeout_returns_408() {
  let mut router = Router::new();
  router.timeout(Duration::from_millis(10));
  router.route(Method::GET, "/slow", |_req: Request| async {
    tokio::time::sleep(Duration::from_millis(100)).await;
    "done"
  });

  let resp = router.dispatch(make_req(Method::GET, "/slow")).await;
  assert_eq!(resp.status(), StatusCode::REQUEST_TIMEOUT);
}

#[tokio::test]
async fn router_timeout_fallback() {
  let mut router = Router::new();
  router.timeout(Duration::from_millis(10));
  router.timeout_fallback(|_req: Request| async {
    (StatusCode::GATEWAY_TIMEOUT, "Too slow!")
  });
  router.route(Method::GET, "/slow", |_req: Request| async {
    tokio::time::sleep(Duration::from_millis(100)).await;
    "done"
  });

  let resp = router.dispatch(make_req(Method::GET, "/slow")).await;
  assert_eq!(resp.status(), StatusCode::GATEWAY_TIMEOUT);
  assert_eq!(body_str(resp).await, "Too slow!");
}

#[tokio::test]
async fn error_handler_transforms_5xx() {
  let mut router = Router::new();
  router.route(Method::GET, "/error", |_req: Request| async {
    (StatusCode::INTERNAL_SERVER_ERROR, "oops")
  });
  router.error_handler(|resp| {
    let status = resp.status();
    let mut new_resp = http::Response::new(TakoBody::from(
      format!("{{\"error\":\"{}\"}}", status.as_u16()),
    ));
    *new_resp.status_mut() = status;
    new_resp.headers_mut().insert(
      http::header::CONTENT_TYPE,
      "application/json".parse().unwrap(),
    );
    new_resp
  });

  let resp = router.dispatch(make_req(Method::GET, "/error")).await;
  assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
  assert_eq!(
    resp.headers().get("content-type").unwrap(),
    "application/json"
  );
  assert_eq!(body_str(resp).await, "{\"error\":\"500\"}");
}

#[tokio::test]
async fn error_handler_ignores_non_5xx() {
  let mut router = Router::new();
  router.route(Method::GET, "/ok", |_req: Request| async { "ok" });
  router.error_handler(|_resp| {
    let mut r = http::Response::new(TakoBody::from("transformed"));
    *r.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    r
  });

  let resp = router.dispatch(make_req(Method::GET, "/ok")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "ok");
}

#[tokio::test]
async fn merge_routers() {
  let mut sub = Router::new();
  sub.route(Method::GET, "/sub", |_req: Request| async { "sub" });

  let mut main = Router::new();
  main.route(Method::GET, "/main", |_req: Request| async { "main" });
  main.merge(sub);

  let resp = main.dispatch(make_req(Method::GET, "/main")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "main");

  let resp = main.dispatch(make_req(Method::GET, "/sub")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "sub");
}

#[tokio::test]
async fn multiple_routes_different_methods() {
  let mut router = Router::new();
  router.route(Method::GET, "/item", |_req: Request| async { "get" });
  router.route(Method::POST, "/item", |_req: Request| async { "post" });

  let resp_get = router.dispatch(make_req(Method::GET, "/item")).await;
  assert_eq!(resp_get.status(), StatusCode::OK);
  assert_eq!(body_str(resp_get).await, "get");

  let resp_post = router.dispatch(make_req(Method::POST, "/item")).await;
  assert_eq!(resp_post.status(), StatusCode::OK);
  assert_eq!(body_str(resp_post).await, "post");
}
