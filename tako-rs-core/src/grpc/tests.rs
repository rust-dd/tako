use std::convert::Infallible;
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use http::HeaderMap;
use http_body::Body;
use http_body::Frame;
use http_body_util::BodyExt;
use prost::Message;

use super::*;
use crate::body::TakoBody;
use crate::extractors::FromRequest;
use crate::extractors::FromRequestParts;
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

async fn inbound(chunks: Vec<Vec<u8>>) -> Vec<Result<i64, String>> {
  let stream = GrpcClientStream::<Number>::from_request(&mut request(chunks))
    .await
    .unwrap();
  stream
    .map(|item| item.map(|n| n.value).map_err(|e| format!("{e:?}")))
    .collect()
    .await
}

#[tokio::test]
async fn server_stream_sends_each_message_then_ok_trailers() {
  let mut metadata = HeaderMap::new();
  metadata.append("x-trace", "a".parse().unwrap());
  metadata.append("x-trace", "b".parse().unwrap());
  let items = futures_util::stream::iter([Ok(Number { value: 1 }), Ok(Number { value: 2 })]);
  let resp = GrpcServerStream::new(items)
    .with_metadata(metadata)
    .into_response();

  assert_eq!(resp.headers()["content-type"], "application/grpc");
  assert_eq!(resp.headers().get_all("x-trace").iter().count(), 2);
  assert!(!resp.headers().contains_key("grpc-status"));
  let frames = frames(resp.into_body()).await;
  assert_eq!(frames.len(), 2);
  assert_eq!(messages(&frames[0]), [1, 2]);
  assert_eq!(status(&frames[1]), ("0", None));
}

#[tokio::test]
async fn server_stream_batches_ready_messages_up_to_the_limit() {
  let items = futures_util::stream::iter(0..10_000).map(|value| Ok(Number { value }));
  let frames = frames(GrpcServerStream::new(items).into_response().into_body()).await;

  let (last, data) = frames.split_last().unwrap();
  assert_eq!(status(last), ("0", None));
  assert!((2..10).contains(&data.len()), "{} DATA frames", data.len());
  for frame in data {
    assert!(frame.data_ref().unwrap().len() < super::streaming::BATCH_LIMIT + 16);
  }
  let values: Vec<i64> = data.iter().flat_map(messages).collect();
  assert_eq!(values, (0..10_000).collect::<Vec<_>>());
}

#[tokio::test]
async fn server_stream_ends_at_the_first_error() {
  let items = futures_util::stream::iter([
    Ok(Number { value: 1 }),
    Err(GrpcStatus::error(GrpcStatusCode::InvalidArgument, "bad é")),
    Ok(Number { value: 2 }),
  ]);
  let frames = frames(GrpcServerStream::new(items).into_response().into_body()).await;

  assert_eq!(frames.len(), 2);
  assert_eq!(messages(&frames[0]), [1]);
  assert_eq!(status(&frames[1]), ("3", Some("bad %C3%A9")));
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
  for resp in [
    GrpcResponse::<Number>::error(GrpcStatusCode::NotFound, "missing").into_response(),
    GrpcStatus::error(GrpcStatusCode::NotFound, "missing").into_response(),
  ] {
    assert_eq!(resp.headers()["grpc-status"], "5");
    assert_eq!(resp.headers()["grpc-message"], "missing");
    assert!(resp.body().is_end_stream());
  }
}

#[tokio::test]
async fn client_stream_reassembles_split_and_batched_frames() {
  let (one, two, three) = (frame(1), frame(2), frame(3));
  let batched = [&one[3..], &two[..], &three[..2]].concat();
  let chunks = vec![one[..3].to_vec(), batched, three[2..].to_vec()];
  assert_eq!(inbound(chunks).await, [Ok(1), Ok(2), Ok(3)]);
}

#[tokio::test]
async fn client_stream_reports_a_truncated_message_once() {
  let chunks = vec![frame(1), frame(2)[..4].to_vec()];
  assert_eq!(
    inbound(chunks).await,
    [Ok(1), Err("InvalidFrame".to_owned())]
  );
}

#[tokio::test]
async fn client_stream_ends_after_oversized_or_compressed_frames() {
  let oversized = ((MAX_GRPC_MESSAGE_SIZE + 1) as u32).to_be_bytes();
  let chunks = vec![[&[0][..], &oversized].concat(), frame(1)];
  assert_eq!(inbound(chunks).await, [Err("MessageTooLarge".to_owned())]);

  let mut compressed = frame(1);
  compressed[0] = 1;
  assert_eq!(
    inbound(vec![compressed, frame(2)]).await,
    [Err("CompressionUnsupported".to_owned())]
  );
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

#[tokio::test]
async fn deadline_extractor_reads_grpc_timeout() {
  let mut req = request(Vec::new());
  req
    .headers_mut()
    .insert("grpc-timeout", "100m".parse().unwrap());
  let stored = read_grpc_deadline(&mut req).unwrap();
  let (mut parts, _) = req.into_parts();
  let extracted = Option::<GrpcDeadline>::from_request_parts(&mut parts)
    .await
    .unwrap()
    .unwrap();
  assert_eq!(extracted.0, stored.0);
  assert!(extracted.remaining() <= Duration::from_millis(100));

  let (mut parts, _) = request(Vec::new()).into_parts();
  let missing = Option::<GrpcDeadline>::from_request_parts(&mut parts)
    .await
    .unwrap();
  assert!(missing.is_none());
}
