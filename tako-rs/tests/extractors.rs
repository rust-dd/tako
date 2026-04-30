use http::Method;
use http::StatusCode;
use http_body_util::BodyExt;
use serde::Deserialize;
use tako::body::TakoBody;
use tako::extractors::FromRequest;
use tako::extractors::FromRequestParts;
use tako::responder::Responder;

async fn body_str(resp: tako::types::Response) -> String {
  let bytes = resp.into_body().collect().await.unwrap().to_bytes();
  String::from_utf8(bytes.to_vec()).unwrap()
}

#[derive(Debug, Deserialize, serde::Serialize, PartialEq)]
struct TestUser {
  name: String,
  age: u32,
}

#[tokio::test]
async fn json_valid_extraction() {
  use tako::extractors::json::Json;

  let mut req = http::Request::builder()
    .method(Method::POST)
    .uri("/api")
    .header("content-type", "application/json")
    .body(TakoBody::from(r#"{"name":"Alice","age":30}"#))
    .unwrap();

  let Json(user) = Json::<TestUser>::from_request(&mut req).await.unwrap();
  assert_eq!(user.name, "Alice");
  assert_eq!(user.age, 30);
}

#[tokio::test]
async fn json_missing_content_type() {
  use tako::extractors::json::Json;
  use tako::extractors::json::JsonError;

  let mut req = http::Request::builder()
    .method(Method::POST)
    .uri("/api")
    .body(TakoBody::from(r#"{"name":"Alice","age":30}"#))
    .unwrap();

  let result = Json::<TestUser>::from_request(&mut req).await;
  match result {
    Err(JsonError::InvalidContentType) => {
      assert_eq!(
        JsonError::InvalidContentType.into_response().status(),
        StatusCode::BAD_REQUEST,
      );
    }
    _ => panic!("expected InvalidContentType"),
  }
}

#[tokio::test]
async fn json_wrong_content_type() {
  use tako::extractors::json::Json;
  use tako::extractors::json::JsonError;

  let mut req = http::Request::builder()
    .method(Method::POST)
    .uri("/api")
    .header("content-type", "text/plain")
    .body(TakoBody::from(r#"{"name":"Alice","age":30}"#))
    .unwrap();

  let result = Json::<TestUser>::from_request(&mut req).await;
  assert!(matches!(result, Err(JsonError::InvalidContentType)));
}

#[tokio::test]
async fn json_invalid_body() {
  use tako::extractors::json::Json;
  use tako::extractors::json::JsonError;

  let mut req = http::Request::builder()
    .method(Method::POST)
    .uri("/api")
    .header("content-type", "application/json")
    .body(TakoBody::from("not json"))
    .unwrap();

  let result = Json::<TestUser>::from_request(&mut req).await;
  match result {
    Err(JsonError::DeserializationError(_)) => {
      // also check that DeserializationError produces 400
      let resp = JsonError::DeserializationError("test".into()).into_response();
      assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
    _ => panic!("expected DeserializationError"),
  }
}

#[tokio::test]
async fn json_response_serialization() {
  use tako::extractors::json::Json;

  let resp = Json(TestUser {
    name: "Bob".to_string(),
    age: 25,
  })
  .into_response();

  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(
    resp.headers().get("content-type").unwrap(),
    "application/json"
  );
  let body = body_str(resp).await;
  let parsed: TestUser = serde_json::from_str(&body).unwrap();
  assert_eq!(parsed.name, "Bob");
}

#[derive(Debug, Deserialize, PartialEq)]
struct SearchQuery {
  q: String,
  page: Option<u32>,
}

#[tokio::test]
async fn query_valid_extraction() {
  use tako::extractors::query::Query;

  let mut req = http::Request::builder()
    .method(Method::GET)
    .uri("/search?q=rust&page=2")
    .body(TakoBody::empty())
    .unwrap();

  let Query(search) = Query::<SearchQuery>::from_request(&mut req).await.unwrap();
  assert_eq!(search.q, "rust");
  assert_eq!(search.page, Some(2));
}

#[tokio::test]
async fn query_optional_fields() {
  use tako::extractors::query::Query;

  let mut req = http::Request::builder()
    .method(Method::GET)
    .uri("/search?q=rust")
    .body(TakoBody::empty())
    .unwrap();

  let Query(search) = Query::<SearchQuery>::from_request(&mut req).await.unwrap();
  assert_eq!(search.q, "rust");
  assert_eq!(search.page, None);
}

#[tokio::test]
async fn query_missing_required_field() {
  use tako::extractors::query::Query;
  use tako::extractors::query::QueryError;

  let mut req = http::Request::builder()
    .method(Method::GET)
    .uri("/search")
    .body(TakoBody::empty())
    .unwrap();

  let result = Query::<SearchQuery>::from_request(&mut req).await;
  match result {
    Err(QueryError::DeserializationError(_)) => {
      let resp = QueryError::DeserializationError("test".into()).into_response();
      assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
    _ => panic!("expected DeserializationError"),
  }
}

#[tokio::test]
async fn query_from_request_parts() {
  use tako::extractors::query::Query;

  let (mut parts, _body) = http::Request::builder()
    .method(Method::GET)
    .uri("/search?q=hello&page=1")
    .body(())
    .unwrap()
    .into_parts();

  let Query(search) = Query::<SearchQuery>::from_request_parts(&mut parts)
    .await
    .unwrap();
  assert_eq!(search.q, "hello");
  assert_eq!(search.page, Some(1));
}

#[derive(Debug, Deserialize, PartialEq)]
struct LoginForm {
  username: String,
  password: String,
}

#[tokio::test]
async fn form_valid_extraction() {
  use tako::extractors::form::Form;

  let mut req = http::Request::builder()
    .method(Method::POST)
    .uri("/login")
    .header("content-type", "application/x-www-form-urlencoded")
    .body(TakoBody::from("username=alice&password=secret"))
    .unwrap();

  let Form(login) = Form::<LoginForm>::from_request(&mut req).await.unwrap();
  assert_eq!(login.username, "alice");
  assert_eq!(login.password, "secret");
}

#[tokio::test]
async fn form_missing_content_type() {
  use tako::extractors::form::Form;
  use tako::extractors::form::FormError;

  let mut req = http::Request::builder()
    .method(Method::POST)
    .uri("/login")
    .body(TakoBody::from("username=alice&password=secret"))
    .unwrap();

  let result = Form::<LoginForm>::from_request(&mut req).await;
  match result {
    Err(FormError::InvalidContentType) => {
      assert_eq!(
        FormError::InvalidContentType.into_response().status(),
        StatusCode::BAD_REQUEST,
      );
    }
    _ => panic!("expected InvalidContentType"),
  }
}

#[tokio::test]
async fn form_wrong_content_type() {
  use tako::extractors::form::Form;
  use tako::extractors::form::FormError;

  let mut req = http::Request::builder()
    .method(Method::POST)
    .uri("/login")
    .header("content-type", "application/json")
    .body(TakoBody::from("username=alice&password=secret"))
    .unwrap();

  let result = Form::<LoginForm>::from_request(&mut req).await;
  assert!(matches!(result, Err(FormError::InvalidContentType)));
}

#[tokio::test]
async fn form_deserialization_error() {
  use tako::extractors::form::Form;
  use tako::extractors::form::FormError;

  let mut req = http::Request::builder()
    .method(Method::POST)
    .uri("/login")
    .header("content-type", "application/x-www-form-urlencoded")
    .body(TakoBody::from("username=alice"))
    .unwrap();

  let result = Form::<LoginForm>::from_request(&mut req).await;
  assert!(matches!(result, Err(FormError::DeserializationError(_))));
}

#[tokio::test]
async fn accept_prefers_json() {
  use tako::extractors::accept::Accept;

  let (mut parts, _) = http::Request::builder()
    .header("accept", "application/json, text/html;q=0.9")
    .body(())
    .unwrap()
    .into_parts();

  let accept = Accept::from_request_parts(&mut parts).await.unwrap();
  assert!(accept.prefers("application/json"));
  assert!(!accept.prefers("text/html"));
  assert!(accept.accepts("text/html"));
}

#[tokio::test]
async fn accept_no_header_defaults_to_wildcard() {
  use tako::extractors::accept::Accept;

  let (mut parts, _) = http::Request::builder().body(()).unwrap().into_parts();

  let accept = Accept::from_request_parts(&mut parts).await.unwrap();
  assert!(accept.prefers("anything"));
  assert!(accept.accepts("application/json"));
}

#[tokio::test]
async fn accept_wildcard_subtype() {
  use tako::extractors::accept::Accept;

  let (mut parts, _) = http::Request::builder()
    .header("accept", "text/*")
    .body(())
    .unwrap()
    .into_parts();

  let accept = Accept::from_request_parts(&mut parts).await.unwrap();
  assert!(accept.accepts("text/html"));
  assert!(accept.accepts("text/plain"));
  assert!(!accept.accepts("application/json"));
}

#[tokio::test]
async fn accept_quality_ordering() {
  use tako::extractors::accept::Accept;

  let (mut parts, _) = http::Request::builder()
    .header("accept", "text/html;q=0.5, application/json;q=0.9")
    .body(())
    .unwrap()
    .into_parts();

  let accept = Accept::from_request_parts(&mut parts).await.unwrap();
  assert_eq!(accept.preferred(), Some("application/json"));
  let types = accept.types();
  assert_eq!(types, vec!["application/json", "text/html"]);
}
