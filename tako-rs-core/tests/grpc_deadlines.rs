#![cfg(feature = "grpc")]

use tako_rs_core::body::TakoBody;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::grpc::GrpcDeadline;
use tako_rs_core::grpc::read_grpc_deadline;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;

fn request() -> Request {
  http::Request::builder()
    .uri("/nested/check")
    .header("grpc-timeout", "100m")
    .body(TakoBody::empty())
    .unwrap()
}

#[tokio::test]
async fn request_and_parts_extractors_share_the_first_absolute_deadline() {
  let mut request = request();
  let first = Option::<GrpcDeadline>::from_request(&mut request)
    .await
    .unwrap()
    .unwrap();
  let second = Option::<GrpcDeadline>::from_request(&mut request)
    .await
    .unwrap()
    .unwrap();
  assert_eq!(first.0, second.0);
  let (mut parts, body) = request.into_parts();
  for _ in 0..2 {
    let deadline = Option::<GrpcDeadline>::from_request_parts(&mut parts)
      .await
      .unwrap()
      .unwrap();
    assert_eq!(deadline.0, first.0);
  }
  let mut request = Request::from_parts(parts, body);
  assert_eq!(read_grpc_deadline(&mut request).unwrap().0, first.0);
}

#[tokio::test]
async fn parts_extraction_stores_the_deadline_for_request_extraction() {
  let (mut parts, body) = request().into_parts();
  let first = Option::<GrpcDeadline>::from_request_parts(&mut parts)
    .await
    .unwrap()
    .unwrap();
  let second = Option::<GrpcDeadline>::from_request_parts(&mut parts)
    .await
    .unwrap()
    .unwrap();
  assert_eq!(first.0, second.0);
  let mut request = Request::from_parts(parts, body);
  let deadline = Option::<GrpcDeadline>::from_request(&mut request)
    .await
    .unwrap()
    .unwrap();
  assert_eq!(deadline.0, first.0);
}

#[derive(Clone)]
struct IngressDeadline(std::time::Instant);

#[tokio::test]
async fn middleware_and_nested_routing_preserve_the_first_deadline() {
  let mut nested = Router::new();
  nested.get("/check", |mut request: Request| async move {
    let first = request.extensions().get::<IngressDeadline>().unwrap().0;
    let deadline = Option::<GrpcDeadline>::from_request(&mut request)
      .await
      .unwrap()
      .unwrap();
    assert_eq!(deadline.0, first);
    http::StatusCode::NO_CONTENT
  });
  let mut router = Router::new();
  router.middleware(|mut request, next| async move {
    let deadline = Option::<GrpcDeadline>::from_request(&mut request)
      .await
      .unwrap()
      .unwrap();
    request.extensions_mut().insert(IngressDeadline(deadline.0));
    next.run(request).await
  });
  router.nest("/nested", nested);
  assert_eq!(
    router.dispatch(request()).await.status(),
    http::StatusCode::NO_CONTENT
  );
}
