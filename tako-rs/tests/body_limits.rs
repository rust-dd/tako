use bytes::Bytes;
use tako::StatusCode;
use tako::body::TakoBody;
use tako::extractors::FromRequest;
use tako::extractors::body::BodyLimit;
use tako::responder::Responder;
use tako::types::Request;

fn limited(content_type: &str) -> Request {
  let mut req = http::Request::builder()
    .method("POST")
    .header("content-type", content_type)
    .body(TakoBody::from("\"abcdef\""))
    .unwrap();
  req.extensions_mut().insert(BodyLimit(Some(2)));
  req
}

#[tokio::test]
async fn form_and_nested_middleware_limits_report_413() {
  use tako::extractors::form::Form;
  let error = Form::<std::collections::HashMap<String, String>>::from_request(&mut limited(
    "application/x-www-form-urlencoded",
  ))
  .await
  .err()
  .unwrap();
  assert_eq!(
    error.into_response().status(),
    StatusCode::PAYLOAD_TOO_LARGE
  );
  let mut req = Request::new(TakoBody::new(http_body_util::Limited::new(
    TakoBody::from("12345"),
    2,
  )));
  req.extensions_mut().insert(BodyLimit(None));
  let error = Bytes::from_request(&mut req).await.unwrap_err();
  assert_eq!(
    error.into_response().status(),
    StatusCode::PAYLOAD_TOO_LARGE
  );
}

#[cfg(feature = "simd-json-impl")]
#[tokio::test]
async fn simd_json_limit_reports_413() {
  let error =
    tako::extractors::simdjson::SimdJson::<String>::from_request(&mut limited("application/json"))
      .await
      .err()
      .unwrap();
  assert_eq!(
    error.into_response().status(),
    StatusCode::PAYLOAD_TOO_LARGE
  );
}

#[cfg(feature = "simd-sonic")]
#[tokio::test]
async fn sonic_json_limit_reports_413() {
  let error =
    tako::extractors::simdjson::SonicJson::<String>::from_request(&mut limited("application/json"))
      .await
      .err()
      .unwrap();
  assert_eq!(
    error.into_response().status(),
    StatusCode::PAYLOAD_TOO_LARGE
  );
}

#[cfg(feature = "zero-copy-extractors")]
#[tokio::test]
async fn borrowed_body_extractors_enforce_limits() {
  use tako::zero_copy_extractors::bytes::BytesBorrowed;
  use tako::zero_copy_extractors::form::FormBorrowed;
  use tako::zero_copy_extractors::json::JsonBorrowed;
  let mut req = limited("application/json");
  let error = BytesBorrowed::from_request(&mut req).await.err().unwrap();
  assert_eq!(
    error.into_response().status(),
    StatusCode::PAYLOAD_TOO_LARGE
  );
  let error = JsonBorrowed::<String>::from_request(&mut limited("application/json"))
    .await
    .err()
    .unwrap();
  assert_eq!(
    error.into_response().status(),
    StatusCode::PAYLOAD_TOO_LARGE
  );
  let error = FormBorrowed::<std::collections::HashMap<String, String>>::from_request(
    &mut limited("application/x-www-form-urlencoded"),
  )
  .await
  .err()
  .unwrap();
  assert_eq!(
    error.into_response().status(),
    StatusCode::PAYLOAD_TOO_LARGE
  );
}
