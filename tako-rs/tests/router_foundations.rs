use std::time::Duration;

use bytes::Bytes;
use http_body_util::BodyExt;
use tako::Method;
use tako::StatusCode;
use tako::body::TakoBody;
use tako::extractors::json::Json;
use tako::extractors::state::State;
use tako::responder::Responder;
use tako::router::Router;
use tako::types::Request;

async fn text(response: tako::types::Response) -> String {
  String::from_utf8(
    response
      .into_body()
      .collect()
      .await
      .unwrap()
      .to_bytes()
      .to_vec(),
  )
  .unwrap()
}

fn request(method: Method, path: &str, body: TakoBody) -> Request {
  http::Request::builder()
    .method(method)
    .uri(path)
    .body(body)
    .unwrap()
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn nested_state_prefers_child_and_preserves_parent_and_siblings() {
  #[derive(Clone)]
  struct Parent(&'static str);
  let mut root = Router::new();
  root
    .with_state(Parent("shared"))
    .with_state(String::from("root"));
  for name in ["a", "b"] {
    let mut child = Router::new();
    child.with_state(name.to_string());
    child.get(
      "/",
      |State(name): State<String>, State(parent): State<Parent>| async move {
        format!("{}:{}", name, parent.0)
      },
    );
    root.nest(&format!("/{name}"), child);
  }
  root.get(
    "/",
    |State(name): State<String>| async move { name.to_string() },
  );
  for (path, expected) in [("/a", "a:shared"), ("/b", "b:shared"), ("/", "root")] {
    assert_eq!(
      text(
        root
          .dispatch(request(Method::GET, path, TakoBody::empty()))
          .await
      )
      .await,
      expected
    );
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn nested_timeout_fallback_receives_original_metadata() {
  let mut child = Router::new();
  child.timeout(Duration::from_millis(5));
  child.timeout_fallback(|req: Request| async move {
    (
      StatusCode::SERVICE_UNAVAILABLE,
      format!(
        "{} {} {}",
        req.method(),
        req.uri(),
        req.headers()["x-id"].to_str().unwrap()
      ),
    )
  });
  child.post("/slow", std::future::pending::<StatusCode>);
  let mut root = Router::new();
  root.nest("/api", child);
  let mut req = request(Method::POST, "/api/slow?q=1", TakoBody::empty());
  req.headers_mut().insert("x-id", "42".parse().unwrap());
  let response = root.dispatch(req).await;
  assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
  assert_eq!(text(response).await, "POST /api/slow?q=1 42");
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn buffering_limits_unknown_length_bodies_and_can_be_overridden() {
  let mut router = Router::new();
  router
    .body_limit(4)
    .post("/", |body: Bytes| async move { body });
  let chunks = futures_util::stream::iter([
    Ok::<_, std::io::Error>(Bytes::from_static(b"123")),
    Ok(Bytes::from_static(b"45")),
  ]);
  let response = router
    .dispatch(request(Method::POST, "/", TakoBody::from_stream(chunks)))
    .await;
  assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
  router.disable_body_limit();
  let response = router
    .dispatch(request(Method::POST, "/", TakoBody::from("12345")))
    .await;
  assert_eq!(text(response).await, "12345");
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn json_limit_is_typed_and_returns_413() {
  let mut router = Router::new();
  router.body_limit(3).post(
    "/",
    |Json(value): Json<serde_json::Value>| async move { value },
  );
  let mut req = request(Method::POST, "/", TakoBody::from("[1,2]"));
  req
    .headers_mut()
    .insert("content-type", "application/json".parse().unwrap());
  assert_eq!(
    router.dispatch(req).await.status(),
    StatusCode::PAYLOAD_TOO_LARGE
  );
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn default_limit_applies_without_router_configuration() {
  let mut router = Router::new();
  router.post("/", |body: String| async move { body });
  let response = router
    .dispatch(request(
      Method::POST,
      "/",
      TakoBody::from(vec![b'a'; 2 * 1024 * 1024 + 1]),
    ))
    .await;
  assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn problem_json_preserves_semantic_headers_and_removes_representation_headers() {
  let mut router = Router::new();
  router.use_problem_json();
  router.get("/", || async {
    let mut response = (StatusCode::TOO_MANY_REQUESTS, "quota exhausted").into_response();
    for (key, value) in [
      ("retry-after", "60"),
      ("set-cookie", "session=abc"),
      ("etag", "old"),
      ("content-length", "15"),
    ] {
      response
        .headers_mut()
        .append(http::HeaderName::from_static(key), value.parse().unwrap());
    }
    response
  });
  let response = router.dispatch(Request::default()).await;
  assert_eq!(response.headers()["retry-after"], "60");
  assert_eq!(response.headers()["set-cookie"], "session=abc");
  assert!(!response.headers().contains_key("etag"));
  assert!(!response.headers().contains_key("content-length"));
  assert!(text(response).await.contains("quota exhausted"));
  let response = router
    .dispatch(request(Method::POST, "/", TakoBody::empty()))
    .await;
  assert_eq!(response.headers()["allow"], "GET, HEAD");
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn result_errors_and_request_aware_formatters_work() {
  let mut router = Router::new();
  router.get("/", || async { Err::<(), _>(StatusCode::UNAUTHORIZED) });
  router.error_handler_with_parts(|parts, mut response| {
    response
      .headers_mut()
      .insert("x-id", parts.headers["x-id"].clone());
    response
  });
  let mut req = Request::default();
  req.headers_mut().insert("x-id", "42".parse().unwrap());
  let response = router.dispatch(req).await;
  assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
  assert_eq!(response.headers()["x-id"], "42");
  let response = anyhow::anyhow!("private database password").into_response();
  assert_eq!(text(response).await, "Internal Server Error");
}

#[test]
fn common_responders_set_content_types() {
  assert_eq!(
    "hello".into_response().headers()["content-type"],
    "text/plain; charset=utf-8"
  );
  assert_eq!(
    (StatusCode::BAD_REQUEST, String::from("bad"))
      .into_response()
      .headers()["content-type"],
    "text/plain; charset=utf-8"
  );
  assert_eq!(
    vec![1_u8].into_response().headers()["content-type"],
    "application/octet-stream"
  );
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn metadata_extractors_work_alone_and_before_a_raw_request() {
  use tako::extractors::accept::Accept;
  let mut router = Router::new();
  router.get("/", |accept: Accept| async move {
    accept.accepts("text/plain").to_string()
  });
  router.post("/", |accept: Accept, request: Request| async move {
    assert!(accept.accepts("text/plain"));
    request.into_body()
  });
  assert_eq!(
    text(router.dispatch(Request::default()).await).await,
    "true"
  );
  assert_eq!(
    text(
      router
        .dispatch(request(Method::POST, "/", TakoBody::from("raw")))
        .await
    )
    .await,
    "raw"
  );
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn verified_claims_extractor_rejects_missing_authentication() {
  use tako::extractors::jwt::JwtClaimsVerified;
  let mut router = Router::new();
  router.get(
    "/",
    |JwtClaimsVerified(claims): JwtClaimsVerified<String>| async move { claims },
  );
  let response = router.dispatch(Request::default()).await;
  assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
