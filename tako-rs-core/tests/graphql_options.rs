#![cfg(feature = "async-graphql")]

use std::sync::Arc;

use async_graphql::http::MultipartOptions;
use tako_rs_core::body::TakoBody;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::graphql::GraphQLOptions;
use tako_rs_core::graphql::GraphQLRequest;
use tako_rs_core::graphql::attach_graphql_options;
use tako_rs_core::router_state::RouterState;
use tako_rs_core::state::get_or_init_state;
use tako_rs_core::types::Request;

fn limited(max_file_size: usize) -> GraphQLOptions {
  GraphQLOptions {
    multipart: MultipartOptions::default().max_file_size(max_file_size),
  }
}

/// A GraphQL multipart upload carrying a 100-byte file.
fn upload() -> Request {
  let operations = r#"{"query":"mutation($f: Upload!) { upload(f: $f) }","variables":{"f":null}}"#;
  let body = format!(
    "--X\r\nContent-Disposition: form-data; name=\"operations\"\r\n\r\n{operations}\r\n\
     --X\r\nContent-Disposition: form-data; name=\"map\"\r\n\r\n{{\"0\":[\"variables.f\"]}}\r\n\
     --X\r\nContent-Disposition: form-data; name=\"0\"; filename=\"a.txt\"\r\n\
     Content-Type: text/plain\r\n\r\n{}\r\n--X--\r\n",
    "x".repeat(100)
  );
  http::Request::builder()
    .method(http::Method::POST)
    .uri("/graphql")
    .header(
      http::header::CONTENT_TYPE,
      "multipart/form-data; boundary=X",
    )
    .body(TakoBody::from(body))
    .unwrap()
}

#[tokio::test]
async fn per_request_options_come_first() {
  let mut req = upload();
  attach_graphql_options(&mut req, limited(1000));
  assert!(GraphQLRequest::from_request(&mut req).await.is_ok());
}

#[tokio::test]
async fn options_in_router_state_apply() {
  let state = Arc::new(RouterState::new());
  state.insert(limited(10));
  let mut req = upload();
  req.extensions_mut().insert(state);
  assert!(GraphQLRequest::from_request(&mut req).await.is_err());
}

#[tokio::test]
async fn process_global_options_apply() {
  get_or_init_state(|| limited(10));
  let mut req = upload();
  assert!(GraphQLRequest::from_request(&mut req).await.is_err());
}
