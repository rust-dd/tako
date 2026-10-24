#![cfg(feature = "async-graphql")]

use std::convert::Infallible;

use async_graphql::BatchRequest;
use async_graphql::http::MultipartOptions;
use bytes::Bytes;
use http::StatusCode;
use tako_rs_core::body::TakoBody;
use tako_rs_core::extractors::body::BodyLimit;
use tako_rs_core::graphql::GraphQLError;
use tako_rs_core::graphql::GraphQLRequest;
use tako_rs_core::graphql::receive_graphql;
use tako_rs_core::graphql::receive_graphql_batch;
use tako_rs_core::responder::Responder;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;

fn request(content_type: &str, body: impl Into<Bytes>) -> Request {
  let bytes = body.into();
  let middle = bytes.len() / 2;
  let stream = futures_util::stream::iter([
    Ok::<_, Infallible>(bytes.slice(..middle)),
    Ok(bytes.slice(middle..)),
  ]);
  http::Request::builder()
    .method("POST")
    .uri("/graphql")
    .header("content-type", content_type)
    .body(TakoBody::from_stream(stream))
    .unwrap()
}

#[tokio::test]
async fn raw_graphql_is_a_single_utf8_query() {
  let mut req = request("Application/GraphQL; charset=utf-8", "{ __typename }");
  let parsed = receive_graphql(&mut req, MultipartOptions::default())
    .await
    .unwrap();
  assert_eq!(parsed.query, "{ __typename }");
  let mut req = request("application/graphql", "{ hello }");
  let parsed = receive_graphql_batch(&mut req, MultipartOptions::default())
    .await
    .unwrap();
  assert!(matches!(parsed, BatchRequest::Single(query) if query.query == "{ hello }"));
  let mut req = request("application/graphql", Bytes::from_static(b"\xff"));
  assert!(matches!(
    receive_graphql(&mut req, MultipartOptions::default()).await,
    Err(GraphQLError::Parse(_))
  ));
}

#[tokio::test]
async fn graphql_router_limits_count_streamed_bytes_despite_content_length() {
  let mut router = Router::new();
  router.body_limit(1);
  router.post("/graphql", |_: GraphQLRequest| async { "accepted" });
  for declared in [None, Some("1")] {
    let mut req = request("application/json", r#"{"query":"{ hello }"}"#);
    if let Some(declared) = declared {
      req
        .headers_mut()
        .insert("content-length", declared.parse().unwrap());
    }
    assert_eq!(
      router.dispatch(req).await.status(),
      StatusCode::PAYLOAD_TOO_LARGE
    );
  }
}

#[tokio::test]
async fn nested_router_and_middleware_can_override_the_shared_cap() {
  for (parent, child, expected) in [
    (Some(1), None, StatusCode::OK),
    (None, Some(1), StatusCode::PAYLOAD_TOO_LARGE),
  ] {
    let mut parent_router = Router::new();
    match parent {
      Some(limit) => parent_router.body_limit(limit),
      None => parent_router.disable_body_limit(),
    };
    let mut child_router = Router::new();
    match child {
      Some(limit) => child_router.body_limit(limit),
      None => child_router.disable_body_limit(),
    };
    child_router.post("/graphql", |_: GraphQLRequest| async { "accepted" });
    parent_router.nest("/child", child_router);
    let mut req = request("application/json", r#"{"query":"{ hello }"}"#);
    *req.uri_mut() = "/child/graphql".parse().unwrap();
    assert_eq!(parent_router.dispatch(req).await.status(), expected);
  }
  let mut router = Router::new();
  router.body_limit(1);
  router.middleware(|mut req, next| async move {
    req.extensions_mut().insert(BodyLimit(None));
    next.run(req).await
  });
  router.post("/graphql", |_: GraphQLRequest| async { "accepted" });
  assert_eq!(
    router
      .dispatch(request("application/json", r#"{"query":"{ hello }"}"#))
      .await
      .status(),
    StatusCode::OK
  );
}

#[tokio::test]
async fn transport_errors_keep_the_original_read_error() {
  let mut req = request("application/json", "");
  *req.body_mut() = TakoBody::from_stream(futures_util::stream::iter([Err::<Bytes, _>(
    std::io::Error::other("broken upload"),
  )]));
  let result = receive_graphql(&mut req, MultipartOptions::default()).await;
  assert!(matches!(result, Err(GraphQLError::BodyRead(message)) if message == "broken upload"));
}

#[tokio::test]
async fn invalid_json_media_type_parameters_keep_the_parse_error() {
  let content_type = "application/json;broken";
  let body = r#"{"query":"{ hello }"}"#;
  let expected = async_graphql::http::receive_batch_body(
    Some(content_type),
    body.as_bytes(),
    MultipartOptions::default(),
  )
  .await
  .unwrap_err();
  let mut req = request(content_type, body);
  assert!(matches!(
    receive_graphql_batch(&mut req, MultipartOptions::default()).await,
    Err(GraphQLError::Parse(message)) if message == expected.to_string()
  ));
}

#[tokio::test]
async fn graphql_default_override_and_disabled_limits_keep_the_protocol_ceiling() {
  let body = format!(r#"{{"query":"{}"}}"#, "x".repeat(2 * 1024 * 1024));
  let mut req = request("application/json", body.clone());
  assert!(matches!(
    receive_graphql(&mut req, MultipartOptions::default()).await,
    Err(GraphQLError::BodyTooLarge)
  ));
  for limit in [BodyLimit(Some(body.len())), BodyLimit(None)] {
    let mut req = request("application/json", body.clone());
    req.extensions_mut().insert(limit);
    assert!(
      receive_graphql(&mut req, MultipartOptions::default())
        .await
        .is_ok()
    );
  }
  let mut req = request("application/json", "x".repeat(4 * 1024 * 1024 + 1));
  req.extensions_mut().insert(BodyLimit(None));
  let error = receive_graphql(&mut req, MultipartOptions::default())
    .await
    .unwrap_err();
  assert!(matches!(error, GraphQLError::BodyTooLarge));
  assert_eq!(
    error.into_response().status(),
    StatusCode::PAYLOAD_TOO_LARGE
  );
}

#[tokio::test]
async fn json_batch_and_error_contract_matches_async_graphql() {
  for body in [
    r#"{"query":"{ hello }","variables":{"a":1}}"#,
    r#"[{"query":"{ a }"},{"query":"{ b }"}]"#,
    "[]",
    "null",
    "{} trailing",
    "[",
    r#"{"query":5}"#,
  ] {
    let expected = async_graphql::http::receive_batch_json(body.as_bytes()).await;
    let mut req = request("application/json", body.to_owned());
    let actual = receive_graphql_batch(&mut req, MultipartOptions::default()).await;
    match (actual, expected) {
      (Ok(actual), Ok(expected)) => {
        assert_eq!(
          matches!(&actual, BatchRequest::Batch(_)),
          matches!(&expected, BatchRequest::Batch(_))
        );
        assert_eq!(
          actual
            .iter()
            .map(|query| (&query.query, &query.variables))
            .collect::<Vec<_>>(),
          expected
            .iter()
            .map(|query| (&query.query, &query.variables))
            .collect::<Vec<_>>()
        );
      }
      (Err(GraphQLError::Parse(actual)), Err(expected)) => assert_eq!(actual, expected.to_string()),
      (actual, expected) => panic!("{body}: actual={actual:?}, expected={expected:?}"),
    }
  }
  let mut req = request("application/json", r#"[{"query":"{ hello }"}]"#);
  assert!(
    matches!(receive_graphql(&mut req, MultipartOptions::default()).await, Err(GraphQLError::Parse(message)) if message == "Batch requests are not supported")
  );
}
