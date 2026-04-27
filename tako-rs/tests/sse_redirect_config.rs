use http::StatusCode;
use http_body_util::BodyExt;
use tako::responder::Responder;

async fn body_str(resp: tako::types::Response) -> String {
  let bytes = resp.into_body().collect().await.unwrap().to_bytes();
  String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn sse_response_headers() {
  use bytes::Bytes;
  use futures_util::stream;
  use tako::sse::Sse;

  let sse = Sse::new(stream::iter(vec![Bytes::from("hello")]));
  let resp = sse.into_response();

  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(
    resp.headers().get("content-type").unwrap(),
    "text/event-stream"
  );
  assert_eq!(resp.headers().get("cache-control").unwrap(), "no-cache");
  assert_eq!(resp.headers().get("connection").unwrap(), "keep-alive");
}

#[tokio::test]
async fn sse_body_format() {
  use bytes::Bytes;
  use futures_util::stream;
  use tako::sse::Sse;

  let sse = Sse::new(stream::iter(vec![
    Bytes::from("hello"),
    Bytes::from("world"),
  ]));
  let resp = sse.into_response();

  let body = body_str(resp).await;
  assert!(body.contains("data: hello\n\n"));
  assert!(body.contains("data: world\n\n"));
}

#[tokio::test]
async fn redirect_found() {
  let resp = tako::redirect::found("/home").into_response();
  assert_eq!(resp.status(), StatusCode::FOUND);
  assert_eq!(resp.headers().get("location").unwrap(), "/home");
}

#[tokio::test]
async fn redirect_see_other() {
  let resp = tako::redirect::see_other("/login").into_response();
  assert_eq!(resp.status(), StatusCode::SEE_OTHER);
  assert_eq!(resp.headers().get("location").unwrap(), "/login");
}

#[tokio::test]
async fn redirect_temporary() {
  let resp = tako::redirect::temporary("/new").into_response();
  assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
  assert_eq!(resp.headers().get("location").unwrap(), "/new");
}

#[tokio::test]
async fn redirect_permanent_moved() {
  let resp = tako::redirect::permanent_moved("/forever").into_response();
  assert_eq!(resp.status(), StatusCode::MOVED_PERMANENTLY);
  assert_eq!(resp.headers().get("location").unwrap(), "/forever");
}

#[tokio::test]
async fn redirect_permanent() {
  let resp = tako::redirect::permanent("/forever").into_response();
  assert_eq!(resp.status(), StatusCode::PERMANENT_REDIRECT);
  assert_eq!(resp.headers().get("location").unwrap(), "/forever");
}

#[tokio::test]
async fn redirect_struct_methods() {
  use tako::redirect::Redirect;

  let r1 = Redirect::found("/a");
  let r2 = Redirect::with_status("/b", StatusCode::MOVED_PERMANENTLY);

  let resp1 = r1.into_response();
  assert_eq!(resp1.status(), StatusCode::FOUND);
  assert_eq!(resp1.headers().get("location").unwrap(), "/a");

  let resp2 = r2.into_response();
  assert_eq!(resp2.status(), StatusCode::MOVED_PERMANENTLY);
  assert_eq!(resp2.headers().get("location").unwrap(), "/b");
}

#[test]
fn config_new_and_inner() {
  use tako::config::Config;

  let config = Config::new(42u32);
  assert_eq!(*config.inner(), 42);
  assert_eq!(config.into_inner(), 42);
}

#[test]
fn config_error_display() {
  use tako::config::ConfigError;

  let err = ConfigError("missing field".to_string());
  assert_eq!(format!("{err}"), "configuration error: missing field");
}
