use std::convert::Infallible;
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use http_body::Body;
use http_body::Frame;
use http_body_util::BodyExt;
use prost::Message;

use super::*;
use crate::body::TakoBody;
use crate::extractors::FromRequest;
use crate::responder::Responder;
use crate::types::Request;

#[derive(Clone, PartialEq, Message)]
struct Number {
  #[prost(int64, tag = "1")]
  value: i64,
}

fn frame(value: i64) -> Vec<u8> {
  grpc_encode(&Number { value })
}

async fn frames(mut body: TakoBody) -> Vec<Frame<Bytes>> {
  let mut frames = Vec::new();
  while let Some(frame) = body.frame().await {
    frames.push(frame.unwrap());
  }
  frames
}

fn messages(frame: &Frame<Bytes>) -> Vec<i64> {
  let mut data = &frame.data_ref().expect("a DATA frame")[..];
  let mut values = Vec::new();
  while !data.is_empty() {
    let end = 5 + u32::from_be_bytes([data[1], data[2], data[3], data[4]]) as usize;
    values.push(grpc_decode::<Number>(&data[..end]).unwrap().0.value);
    data = &data[end..];
  }
  values
}

fn status(frame: &Frame<Bytes>) -> (&str, Option<&str>) {
  let trailers = frame.trailers_ref().expect("a trailers frame");
  (
    trailers["grpc-status"].to_str().unwrap(),
    trailers.get("grpc-message").map(|v| v.to_str().unwrap()),
  )
}

fn request(chunks: Vec<Vec<u8>>) -> Request {
  grpc_request(TakoBody::from_stream(futures_util::stream::iter(
    chunks
      .into_iter()
      .map(|chunk| Ok::<_, Infallible>(Bytes::from(chunk))),
  )))
}

fn grpc_request(body: TakoBody) -> Request {
  let mut req = Request::new(body);
  req.headers_mut().insert(
    http::header::CONTENT_TYPE,
    "application/grpc".parse().unwrap(),
  );
  req
}

#[tokio::test]
async fn unary_reply_sends_status_in_trailers() {
  let resp = GrpcResponse::ok(Number { value: 7 }).into_response();
  assert_eq!(resp.headers()["content-type"], "application/grpc");
  assert!(!resp.headers().contains_key("grpc-status"));
  let frames = frames(resp.into_body()).await;
  assert_eq!(frames.len(), 2);
  assert_eq!(messages(&frames[0]), [7]);
  assert_eq!(status(&frames[1]), ("0", None));
}

#[tokio::test]
async fn errors_are_trailers_only() {
  let resp = GrpcResponse::<Number>::error(GrpcStatusCode::NotFound, "missing").into_response();
  assert_eq!(resp.headers()["grpc-status"], "5");
  assert_eq!(resp.headers()["grpc-message"], "missing");
  assert!(resp.body().is_end_stream());
}

#[tokio::test]
async fn unary_request_decodes_the_first_message() {
  let req = GrpcRequest::<Number>::from_request(&mut request(vec![frame(4), frame(5)]))
    .await
    .unwrap();
  assert_eq!(req.message.value, 4);

  let empty = GrpcRequest::<Number>::from_request(&mut request(Vec::new())).await;
  assert!(matches!(empty, Err(GrpcError::InvalidFrame)));

  let mut plain = request(vec![frame(4)]);
  plain.headers_mut().remove(http::header::CONTENT_TYPE);
  let plain = GrpcRequest::<Number>::from_request(&mut plain).await;
  assert!(matches!(plain, Err(GrpcError::InvalidContentType)));
}

#[tokio::test]
async fn unary_request_rejects_an_oversized_prefix_without_reading_on() {
  let prefix = [
    &[0][..],
    &((MAX_GRPC_MESSAGE_SIZE + 1) as u32).to_be_bytes(),
  ]
  .concat();
  let endless = futures_util::stream::iter([Ok::<_, Infallible>(Bytes::from(prefix))])
    .chain(futures_util::stream::pending());
  let mut req = grpc_request(TakoBody::from_stream(endless));
  let result = tokio::time::timeout(
    Duration::from_secs(1),
    GrpcRequest::<Number>::from_request(&mut req),
  )
  .await
  .expect("the extractor waited for the rest of the body");
  assert!(matches!(result, Err(GrpcError::MessageTooLarge)));
}
