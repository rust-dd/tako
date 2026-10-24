#![cfg(feature = "grpc")]

use std::convert::Infallible;

use bytes::Bytes;
use futures_util::StreamExt;
use prost::Message;
use tako_rs_core::body::TakoBody;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::body::BodyLimit;
use tako_rs_core::grpc::GrpcClientStream;
use tako_rs_core::grpc::GrpcError;
use tako_rs_core::grpc::GrpcRequest;
use tako_rs_core::grpc::grpc_encode;
use tako_rs_core::responder::Responder;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;

#[derive(Clone, PartialEq, Message)]
struct Payload {
  #[prost(string, tag = "1")]
  value: String,
}

fn encoded(value: &str) -> Vec<u8> {
  grpc_encode(&Payload {
    value: value.to_owned(),
  })
}

fn request(chunks: Vec<Vec<u8>>) -> Request {
  let stream = futures_util::stream::iter(
    chunks
      .into_iter()
      .map(|chunk| Ok::<_, Infallible>(Bytes::from(chunk))),
  );
  http::Request::builder()
    .method("POST")
    .uri("/unary")
    .header("content-type", "application/grpc")
    .body(TakoBody::from_stream(stream))
    .unwrap()
}

#[tokio::test]
async fn unary_rejects_second_messages_and_malformed_remainders() {
  for chunks in [
    vec![[encoded("one"), encoded("two")].concat()],
    vec![encoded("one"), encoded("two")],
    vec![encoded("one"), vec![0, 0]],
  ] {
    let result = GrpcRequest::<Payload>::from_request(&mut request(chunks)).await;
    assert!(
      matches!(result, Err(GrpcError::InvalidFrame)),
      "trailing data was accepted"
    );
  }
}

#[tokio::test]
async fn native_extractors_accept_only_protobuf_media_types() {
  for content_type in [
    "application/grpcjunk",
    "application/grpc+json",
    "application/grpc-web",
    "application/grpc-web+proto",
    "application/grpc;broken",
  ] {
    let mut req = request(vec![encoded("one")]);
    req
      .headers_mut()
      .insert("content-type", content_type.parse().unwrap());
    assert!(
      matches!(
        GrpcRequest::<Payload>::from_request(&mut req).await,
        Err(GrpcError::InvalidContentType)
      ),
      "{content_type}"
    );
    let mut req = request(vec![encoded("one")]);
    req
      .headers_mut()
      .insert("content-type", content_type.parse().unwrap());
    assert!(
      matches!(
        GrpcClientStream::<Payload>::from_request(&mut req).await,
        Err(GrpcError::InvalidContentType)
      ),
      "{content_type}"
    );
  }
  for content_type in [
    "application/grpc",
    "application/grpc+proto",
    "Application/Grpc+Proto; charset=utf-8",
  ] {
    let mut req = request(vec![encoded("one")]);
    req
      .headers_mut()
      .insert("content-type", content_type.parse().unwrap());
    let parsed = GrpcRequest::<Payload>::from_request(&mut req)
      .await
      .unwrap();
    assert_eq!(parsed.message.value, "one", "{content_type}");
  }
}

#[tokio::test]
async fn unary_obeys_router_limits_without_trusting_content_length() {
  let mut router = Router::new();
  router.body_limit(8);
  router.post("/unary", |_: GrpcRequest<Payload>| async { "accepted" });
  for declared in [None, Some("1")] {
    let frame = encoded("too large");
    let mut req = request(vec![frame[..3].to_vec(), frame[3..].to_vec()]);
    if let Some(declared) = declared {
      req
        .headers_mut()
        .insert("content-length", declared.parse().unwrap());
    }
    let response = router.dispatch(req).await;
    assert_eq!(
      response
        .headers()
        .get("grpc-status")
        .map(|v| v.to_str().unwrap()),
      Some("8")
    );
  }
}

#[tokio::test]
async fn unary_default_limit_can_be_overridden_or_disabled() {
  let frame = encoded(&"a".repeat(2 * 1024 * 1024));
  let mut default = request(vec![frame.clone()]);
  let error = GrpcRequest::<Payload>::from_request(&mut default)
    .await
    .err()
    .expect("default buffer limit");
  assert_eq!(error.into_response().headers()["grpc-status"], "8");
  for limit in [BodyLimit(Some(frame.len())), BodyLimit(None)] {
    let mut req = request(vec![frame.clone()]);
    req.extensions_mut().insert(limit);
    assert!(GrpcRequest::<Payload>::from_request(&mut req).await.is_ok());
  }
  let mut req = request(vec![vec![0, 0, 64, 0, 1]]);
  req.extensions_mut().insert(BodyLimit(None));
  assert!(matches!(
    GrpcRequest::<Payload>::from_request(&mut req).await,
    Err(GrpcError::MessageTooLarge)
  ));
}

#[tokio::test]
async fn client_stream_keeps_per_message_limits_without_an_aggregate_cap() {
  let mut req = request(vec![encoded("one"), encoded("two")]);
  req.extensions_mut().insert(BodyLimit(Some(1)));
  let mut stream = GrpcClientStream::<Payload>::from_request(&mut req)
    .await
    .unwrap();
  assert_eq!(stream.next().await.unwrap().unwrap().value, "one");
  assert_eq!(stream.next().await.unwrap().unwrap().value, "two");
  assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn transport_errors_keep_the_original_read_error() {
  let mut req = request(Vec::new());
  *req.body_mut() = TakoBody::from_stream(futures_util::stream::iter([Err::<Bytes, _>(
    std::io::Error::other("broken upload"),
  )]));
  assert!(matches!(
    GrpcRequest::<Payload>::from_request(&mut req).await,
    Err(GrpcError::BodyReadError(message)) if message == "broken upload"
  ));
}
