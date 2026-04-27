//! Integration tests for the `#[route]` proc macro + `TypedParams<T>` extractor.

use http::{Method, StatusCode};
use http_body_util::BodyExt;
use tako::body::TakoBody;
use tako::extractors::typed_params::TypedParams;
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

#[route(GET, "/items/{id: u64}/{name: String}/{ratio: f64}", name = "MixedPath")]
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
  assert!(
    body.contains("invalid path param 'id'"),
    "body was: {body}"
  );
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
