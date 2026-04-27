//! Integration tests for the `#[route]` proc macro + `TypedParams<T>` extractor.

use http::Method;
use http::StatusCode;
use http_body_util::BodyExt;
use tako::body::TakoBody;
use tako::delete;
use tako::extractors::typed_params::TypedParams;
use tako::get;
use tako::patch;
use tako::post;
use tako::put;
use tako::responder::Responder;
use tako::route;
use tako::router::Router;
use tako::types::Request;

#[route(GET, "/users/{id: u64}", name = "SinglePath")]
async fn one(TypedParams(p): TypedParams<SinglePath>) -> impl Responder {
  format!("id={}", p.id)
}

#[route(GET, "/users/{id: u64}/posts/{post_id: u64}", name = "MultiPath")]
async fn two(TypedParams(p): TypedParams<MultiPath>) -> impl Responder {
  format!("{} {}", p.id, p.post_id)
}

#[route(
  GET,
  "/items/{id: u64}/{name: String}/{ratio: f64}",
  name = "MixedPath"
)]
async fn mixed(TypedParams(p): TypedParams<MixedPath>) -> impl Responder {
  format!("{} {} {}", p.id, p.name, p.ratio)
}

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
async fn typed_route_single_param_ok() {
  let mut router = Router::new();
  router.route(SinglePath::METHOD, SinglePath::PATH, one);

  let resp = router.dispatch(make_req(Method::GET, "/users/42")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "id=42");
}

#[tokio::test]
async fn typed_route_multi_param_ok() {
  let mut router = Router::new();
  router.route(MultiPath::METHOD, MultiPath::PATH, two);

  let resp = router
    .dispatch(make_req(Method::GET, "/users/7/posts/3"))
    .await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "7 3");
}

#[tokio::test]
async fn typed_route_mixed_types_ok() {
  let mut router = Router::new();
  router.route(MixedPath::METHOD, MixedPath::PATH, mixed);

  let resp = router
    .dispatch(make_req(Method::GET, "/items/9/widget/0.25"))
    .await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "9 widget 0.25");
}

#[tokio::test]
async fn typed_route_invalid_type_returns_400() {
  let mut router = Router::new();
  router.route(SinglePath::METHOD, SinglePath::PATH, one);

  let resp = router.dispatch(make_req(Method::GET, "/users/abc")).await;
  assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
  let body = body_str(resp).await;
  assert!(body.contains("invalid path param 'id'"), "body was: {body}");
}

#[tokio::test]
async fn typed_route_unknown_path_falls_through_to_404() {
  let mut router = Router::new();
  router.route(SinglePath::METHOD, SinglePath::PATH, one);

  let resp = router.dispatch(make_req(Method::GET, "/elsewhere")).await;
  assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn typed_route_consts_match_macro_inputs() {
  assert_eq!(SinglePath::METHOD, Method::GET);
  assert_eq!(SinglePath::PATH, "/users/{id}");
  assert_eq!(MultiPath::PATH, "/users/{id}/posts/{post_id}");
  assert_eq!(MixedPath::PATH, "/items/{id}/{name}/{ratio}");
}

#[route(GET, "/auto/{id: u64}")]
async fn auto_named(TypedParams(p): TypedParams<AutoNamedParams>) -> impl Responder {
  format!("auto={}", p.id)
}

#[tokio::test]
async fn typed_route_auto_struct_name_from_fn() {
  let mut router = Router::new();
  router.route(AutoNamedParams::METHOD, AutoNamedParams::PATH, auto_named);

  let resp = router.dispatch(make_req(Method::GET, "/auto/5")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "auto=5");
  assert_eq!(AutoNamedParams::PATH, "/auto/{id}");
}

#[tokio::test]
async fn typed_route_router_route_chains_middleware() {
  // `router.route(...)` returns Arc<Route>; chain a no-op middleware to
  // confirm the typed-route attribute keeps the standard registration path.
  let mut router = Router::new();
  let route = router.route(SinglePath::METHOD, SinglePath::PATH, one);
  route.middleware(|req, next| async move { next.run(req).await });

  let resp = router.dispatch(make_req(Method::GET, "/users/1")).await;
  assert_eq!(resp.status(), StatusCode::OK);
}

#[get("/sc/get/{id: u64}")]
async fn sc_get(TypedParams(p): TypedParams<ScGetParams>) -> impl Responder {
  format!("get={}", p.id)
}

#[post("/sc/post/{id: u64}")]
async fn sc_post(TypedParams(p): TypedParams<ScPostParams>) -> impl Responder {
  format!("post={}", p.id)
}

#[put("/sc/put/{id: u64}")]
async fn sc_put(TypedParams(p): TypedParams<ScPutParams>) -> impl Responder {
  format!("put={}", p.id)
}

#[delete("/sc/delete/{id: u64}")]
async fn sc_delete(TypedParams(p): TypedParams<ScDeleteParams>) -> impl Responder {
  format!("delete={}", p.id)
}

#[patch("/sc/patch/{id: u64}")]
async fn sc_patch(TypedParams(p): TypedParams<ScPatchParams>) -> impl Responder {
  format!("patch={}", p.id)
}

#[tokio::test]
async fn shortcut_macros_set_method_correctly() {
  assert_eq!(ScGetParams::METHOD, Method::GET);
  assert_eq!(ScPostParams::METHOD, Method::POST);
  assert_eq!(ScPutParams::METHOD, Method::PUT);
  assert_eq!(ScDeleteParams::METHOD, Method::DELETE);
  assert_eq!(ScPatchParams::METHOD, Method::PATCH);

  assert_eq!(ScGetParams::PATH, "/sc/get/{id}");
  assert_eq!(ScPostParams::PATH, "/sc/post/{id}");
}

#[tokio::test]
async fn shortcut_macros_dispatch_each_method() {
  let mut router = Router::new();
  router.route(ScGetParams::METHOD, ScGetParams::PATH, sc_get);
  router.route(ScPostParams::METHOD, ScPostParams::PATH, sc_post);
  router.route(ScPutParams::METHOD, ScPutParams::PATH, sc_put);
  router.route(ScDeleteParams::METHOD, ScDeleteParams::PATH, sc_delete);
  router.route(ScPatchParams::METHOD, ScPatchParams::PATH, sc_patch);

  for (method, uri, want_body) in [
    (Method::GET, "/sc/get/1", "get=1"),
    (Method::POST, "/sc/post/2", "post=2"),
    (Method::PUT, "/sc/put/3", "put=3"),
    (Method::DELETE, "/sc/delete/4", "delete=4"),
    (Method::PATCH, "/sc/patch/5", "patch=5"),
  ] {
    let resp = router.dispatch(make_req(method.clone(), uri)).await;
    assert_eq!(resp.status(), StatusCode::OK, "method {method} on {uri}");
    assert_eq!(body_str(resp).await, want_body);
  }
}

#[get("/sc/named", name = "HealthOk")]
async fn sc_named() -> impl Responder {
  "ok"
}

#[tokio::test]
async fn shortcut_macros_accept_name_override() {
  assert_eq!(HealthOk::METHOD, Method::GET);
  assert_eq!(HealthOk::PATH, "/sc/named");

  let mut router = Router::new();
  router.route(HealthOk::METHOD, HealthOk::PATH, sc_named);
  let resp = router.dispatch(make_req(Method::GET, "/sc/named")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "ok");
}
