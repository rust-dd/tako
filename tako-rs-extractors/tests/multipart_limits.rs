#![cfg(feature = "multipart")]

use std::convert::Infallible;

use bytes::Bytes;
use http::StatusCode;
use tako_rs_core::body::TakoBody;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::body::BodyLimit;
use tako_rs_core::responder::Responder;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;
use tako_rs_extractors::multipart::InMemoryFile;
use tako_rs_extractors::multipart::MultipartConfig;
use tako_rs_extractors::multipart::TakoMultipart;
use tako_rs_extractors::multipart::TakoTypedMultipart;

type Form<'a> = TakoTypedMultipart<'a, serde_json::Value, InMemoryFile>;

fn body(field_size: usize, count: usize) -> Vec<Bytes> {
  let mut parts = (0..count)
    .map(|index| {
      Bytes::from(format!(
        "--X\r\nContent-Disposition: form-data; name=\"field{index}\"\r\n\r\n{}\r\n",
        "a".repeat(field_size)
      ))
    })
    .collect::<Vec<_>>();
  parts.push(Bytes::from_static(b"--X--\r\n"));
  parts
}

fn request(chunks: Vec<Bytes>) -> Request {
  http::Request::builder()
    .method("POST")
    .uri("/upload")
    .header("content-type", "multipart/form-data; boundary=X")
    .body(TakoBody::from_stream(futures_util::stream::iter(
      chunks.into_iter().map(Ok::<_, Infallible>),
    )))
    .unwrap()
}

#[tokio::test]
async fn typed_fields_have_a_finite_default_aggregate_limit() {
  let mut req = request(body(800 * 1024, 3));
  let result = Form::from_request(&mut req).await;
  let error = result
    .err()
    .expect("buffered text fields must have an aggregate limit");
  assert_eq!(
    error.into_response().status(),
    StatusCode::PAYLOAD_TOO_LARGE
  );
}

#[tokio::test]
async fn typed_router_limit_counts_bytes_with_missing_or_false_content_length() {
  let mut router = Router::new();
  router.body_limit(100);
  router.post("/upload", |mut req: Request| async move {
    Form::from_request(&mut req).await.map(|_| StatusCode::OK)
  });
  for declared in [None, Some("1")] {
    let mut req = request(body(80, 2));
    req
      .extensions_mut()
      .insert(MultipartConfig::new().total_size_limit(1000));
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
async fn typed_buffer_limit_override_and_disable_preserve_explicit_multipart_limits() {
  for limit in [BodyLimit(Some(3 * 1024 * 1024)), BodyLimit(None)] {
    let mut req = request(body(800 * 1024, 3));
    req.extensions_mut().insert(limit);
    let form = Form::from_request(&mut req).await.unwrap();
    assert_eq!(form.data["field2"].as_str().unwrap().len(), 800 * 1024);
  }
  let mut req = request(body(80, 2));
  req.extensions_mut().insert(BodyLimit(None));
  req
    .extensions_mut()
    .insert(MultipartConfig::new().total_size_limit(100));
  let error = Form::from_request(&mut req)
    .await
    .err()
    .expect("multipart limit still applies");
  assert_eq!(
    error.into_response().status(),
    StatusCode::PAYLOAD_TOO_LARGE
  );
}

#[tokio::test]
async fn raw_multipart_stream_uses_its_own_explicit_total_limit() {
  let mut req = request(body(80, 2));
  req.extensions_mut().insert(BodyLimit(Some(1)));
  let TakoMultipart(mut fields) = TakoMultipart::from_request(&mut req).await.unwrap();
  let mut count = 0;
  while let Some(field) = fields.next_field().await.unwrap() {
    assert_eq!(field.bytes().await.unwrap().len(), 80);
    count += 1;
  }
  assert_eq!(count, 2);
}
