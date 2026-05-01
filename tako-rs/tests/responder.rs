use http::StatusCode;
use http_body_util::BodyExt;
use tako::body::TakoBody;
use tako::responder::NOT_FOUND;
use tako::responder::Responder;
use tako::responder::StaticHeaders;

async fn body_str(resp: tako::types::Response) -> String {
  let bytes = resp.into_body().collect().await.unwrap().to_bytes();
  String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn static_str_response() {
  let resp = "hello".into_response();
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "hello");
}

#[tokio::test]
async fn string_response() {
  let resp = "world".to_string().into_response();
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "world");
}

#[tokio::test]
async fn unit_response() {
  let resp = ().into_response();
  assert_eq!(resp.status(), StatusCode::OK);
  let body = resp.into_body().collect().await.unwrap().to_bytes();
  assert!(body.is_empty());
}

#[tokio::test]
async fn status_tuple_response() {
  let resp = (StatusCode::NOT_FOUND, "Not Found").into_response();
  assert_eq!(resp.status(), StatusCode::NOT_FOUND);
  assert_eq!(body_str(resp).await, "Not Found");
}

#[tokio::test]
async fn anyhow_ok_response() {
  let resp = anyhow::Ok("ok").into_response();
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "ok");
}

#[tokio::test]
async fn anyhow_err_response() {
  let resp = anyhow::Result::<String>::Err(anyhow::anyhow!("bad")).into_response();
  assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
  assert_eq!(
    resp.headers().get("content-type").unwrap(),
    "text/plain; charset=utf-8"
  );
  assert_eq!(body_str(resp).await, "bad");
}

#[tokio::test]
async fn not_found_constant() {
  let resp = NOT_FOUND.into_response();
  assert_eq!(resp.status(), StatusCode::NOT_FOUND);
  assert_eq!(body_str(resp).await, "Not Found");
}

#[tokio::test]
async fn static_headers_response() {
  let resp = (
    StatusCode::NO_CONTENT,
    StaticHeaders([(http::header::CACHE_CONTROL, "no-store")]),
  )
    .into_response();
  assert_eq!(resp.status(), StatusCode::NO_CONTENT);
  assert_eq!(resp.headers().get("cache-control").unwrap(), "no-store");
}

#[tokio::test]
async fn takobody_response() {
  let resp = TakoBody::from("raw body").into_response();
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "raw body");
}

#[tokio::test]
async fn response_passthrough() {
  let orig = http::Response::builder()
    .status(StatusCode::CREATED)
    .header("x-custom", "yes")
    .body(TakoBody::from("created"))
    .unwrap();

  let resp = orig.into_response();
  assert_eq!(resp.status(), StatusCode::CREATED);
  assert_eq!(resp.headers().get("x-custom").unwrap(), "yes");
  assert_eq!(body_str(resp).await, "created");
}
