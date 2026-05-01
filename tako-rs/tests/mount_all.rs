//! End-to-end test for `router.mount_all()`. Each integration test file
//! compiles to its own test binary, so the global `TAKO_ROUTES` slice in
//! this binary contains *only* the routes declared below — that's what
//! makes asserting the registered set tractable.

use http::Method;
use http::StatusCode;
use http_body_util::BodyExt;
use tako::body::TakoBody;
use tako::delete;
use tako::extractors::typed_params::TypedParams;
use tako::get;
use tako::post;
use tako::responder::Responder;
use tako::router::Router;
use tako::types::Request;

#[get("/m/users/{id: u64}")]
async fn m_get_user(TypedParams(p): TypedParams<MGetUserParams>) -> impl Responder {
  format!("user={}", p.id)
}

#[post("/m/users")]
async fn m_create_user() -> impl Responder {
  "created"
}

#[delete("/m/users/{id: u64}")]
async fn m_delete_user(TypedParams(p): TypedParams<MDeleteUserParams>) -> impl Responder {
  format!("deleted={}", p.id)
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
async fn mount_all_registers_every_attribute_route() {
  let mut router = Router::new();
  router.mount_all();

  let resp = router.dispatch(make_req(Method::GET, "/m/users/42")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "user=42");

  let resp = router.dispatch(make_req(Method::POST, "/m/users")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "created");

  let resp = router
    .dispatch(make_req(Method::DELETE, "/m/users/7"))
    .await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "deleted=7");
}

#[tokio::test]
async fn mount_all_returns_self_for_chaining() {
  let mut router = Router::new();
  // Returning `&mut Self` lets callers chain a fallback / middleware after
  // the bulk registration in one expression.
  let _: &mut Router = router.mount_all();
}
