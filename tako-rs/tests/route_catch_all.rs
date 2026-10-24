use http_body_util::BodyExt;
use tako::body::TakoBody;
use tako::extractors::path::Path;
use tako::router::Router;

#[tako::get("/files/{*rest}")]
async fn file(Path(rest): Path<String>) -> String {
  rest
}

#[tokio::test]
async fn macro_catch_all_dispatches_a_multi_segment_suffix() {
  let mut router = Router::new();
  router.mount_all();
  let request = http::Request::builder()
    .uri("/files/images/2026/photo.png")
    .body(TakoBody::empty())
    .unwrap();
  let response = router.dispatch(request).await;
  assert_eq!(response.status(), http::StatusCode::OK);
  assert_eq!(
    response.into_body().collect().await.unwrap().to_bytes(),
    "images/2026/photo.png"
  );
}
