#![cfg(feature = "async-graphql")]

use async_graphql::http::MultipartOptions;
use http::StatusCode;
use http_body_util::BodyExt;
use tako_rs_core::body::TakoBody;
use tako_rs_core::graphql::GraphQLError;
use tako_rs_core::graphql::receive_graphql;
use tako_rs_core::responder::Responder;

#[tokio::test]
async fn receive_graphql_rejects_unsupported_types_before_reading() {
  for content_type in [None, Some("text/plain")] {
    let mut builder = http::Request::builder().method("POST");
    if let Some(content_type) = content_type {
      builder = builder.header("content-type", content_type);
    }
    let mut request = builder
      .body(TakoBody::from(r#"{"query":"{ hello }"}"#))
      .unwrap();

    let error = receive_graphql(&mut request, MultipartOptions::default())
      .await
      .expect_err("unsupported content types must be rejected");
    assert!(matches!(error, GraphQLError::UnsupportedMediaType(_)));
    assert_eq!(
      error.into_response().status(),
      StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    let remaining = request.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(remaining, r#"{"query":"{ hello }"}"#);
  }
}

#[tokio::test]
async fn receive_graphql_rejects_empty_graphql_bodies() {
  let mut request = http::Request::builder()
    .method("POST")
    .header("content-type", "application/graphql")
    .body(TakoBody::empty())
    .unwrap();

  let error = receive_graphql(&mut request, MultipartOptions::default())
    .await
    .expect_err("an empty GraphQL body must be rejected");
  assert!(matches!(error, GraphQLError::Parse(_)));
  assert_eq!(error.into_response().status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn receive_graphql_accepts_get_queries_and_json_bodies() {
  let mut get = http::Request::builder()
    .uri("/graphql?query=%7B%20hello%20%7D")
    .body(TakoBody::empty())
    .unwrap();
  let mut post = http::Request::builder()
    .method("POST")
    .header("content-type", "application/json; charset=utf-8")
    .body(TakoBody::from(r#"{"query":"{ hello }"}"#))
    .unwrap();

  for request in [&mut get, &mut post] {
    let parsed = receive_graphql(request, MultipartOptions::default())
      .await
      .unwrap();
    assert_eq!(parsed.query, "{ hello }");
  }
}
