use bytes::Bytes;
use futures_util::FutureExt;
use futures_util::StreamExt;
use http_body::Frame;
use prost::Message;

use super::grpc_request;
use crate::body::TakoBody;
use crate::extractors::FromRequest;
use crate::grpc::GrpcError;
use crate::grpc::GrpcRequest;
use crate::grpc::framing::FrameDecoder;

#[derive(Clone, PartialEq, Message)]
struct Payload {
  #[prost(bytes = "bytes", tag = "1")]
  data: Bytes,
}

const PAYLOAD_FRAME: &[u8] = &[0, 0, 0, 0, 5, 0x0a, 3, b'f', b'o', b'o'];

fn decoder(chunks: Vec<Bytes>) -> FrameDecoder<Payload> {
  FrameDecoder::new(TakoBody::from_stream(futures_util::stream::iter(
    chunks.into_iter().map(Ok::<_, std::io::Error>),
  )))
}

#[tokio::test]
async fn unary_bytes_payload_shares_the_incoming_chunk() {
  let chunk = Bytes::copy_from_slice(PAYLOAD_FRAME);
  let mut request = grpc_request(TakoBody::from(chunk.clone()));
  let message = GrpcRequest::<Payload>::from_request(&mut request)
    .await
    .unwrap()
    .message;

  assert_eq!(message.data.as_ref(), b"foo");
  assert_eq!(message.data.as_ptr(), chunk[7..].as_ptr());
}

#[tokio::test]
async fn batched_bytes_payloads_share_the_incoming_chunk() {
  let chunk = Bytes::from(PAYLOAD_FRAME.repeat(3));
  let mut stream = decoder(vec![chunk.clone()]);

  for offset in [7, 17, 27] {
    let message = stream.next().await.unwrap().unwrap();
    assert_eq!(message.data.as_ref(), b"foo");
    assert_eq!(message.data.as_ptr(), chunk[offset..].as_ptr());
  }
  assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn fragmented_message_keeps_following_complete_frames_shared() {
  let chunk = Bytes::from(PAYLOAD_FRAME.repeat(2));
  for split in 1..PAYLOAD_FRAME.len() {
    let mut stream = decoder(vec![chunk.slice(..split), chunk.slice(split..)]);
    let first = stream.next().await.unwrap().unwrap();
    let second = stream.next().await.unwrap().unwrap();

    assert_eq!(first.data.as_ref(), b"foo", "split at {split}");
    assert_eq!(second.data.as_ref(), b"foo", "split at {split}");
    assert_eq!(
      second.data.as_ptr(),
      chunk[17..].as_ptr(),
      "split at {split}"
    );
    assert!(stream.next().await.is_none(), "split at {split}");
  }
}

#[tokio::test]
async fn bytewise_fragments_reassemble_multiple_messages() {
  let chunks = PAYLOAD_FRAME
    .repeat(3)
    .into_iter()
    .flat_map(|byte| [Bytes::new(), Bytes::from(vec![byte])])
    .collect();
  let messages = decoder(chunks).collect::<Vec<_>>().await;

  assert_eq!(messages.len(), 3);
  for message in messages {
    assert_eq!(message.unwrap().data.as_ref(), b"foo");
  }
}

#[tokio::test]
async fn fragments_survive_pending_body_polls() {
  let chunks = [
    Bytes::from_static(&PAYLOAD_FRAME[..3]),
    Bytes::from_static(&PAYLOAD_FRAME[3..6]),
    Bytes::from_static(&PAYLOAD_FRAME[6..]),
  ];
  let chunks = futures_util::stream::iter(chunks).then(|chunk| async move {
    tokio::task::yield_now().await;
    Ok::<_, std::io::Error>(chunk)
  });
  let mut stream = FrameDecoder::<Payload>::new(TakoBody::from_stream(chunks));

  assert_eq!(stream.next().await.unwrap().unwrap().data.as_ref(), b"foo");
  assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn many_batched_frames_preserve_order() {
  let chunk = (0..10_000).flat_map(super::frame).collect::<Vec<_>>();
  let mut stream = FrameDecoder::<super::Number>::new(TakoBody::from(chunk));

  for value in 0..10_000 {
    assert_eq!(stream.next().await.unwrap().unwrap().value, value);
  }
  assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn empty_messages_decode_at_chunk_boundaries() {
  let mut stream = decoder(vec![
    Bytes::from_static(&[0, 0]),
    Bytes::from_static(&[0, 0, 0, 0, 0, 0, 0, 0]),
  ]);

  for _ in 0..2 {
    assert!(stream.next().await.unwrap().unwrap().data.is_empty());
  }
  assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn every_nonempty_truncated_frame_reports_one_error() {
  for len in 1..PAYLOAD_FRAME.len() {
    let mut stream = decoder(vec![Bytes::from_static(&PAYLOAD_FRAME[..len])]);
    assert!(
      matches!(stream.next().await, Some(Err(GrpcError::InvalidFrame))),
      "truncated at {len}",
    );
    assert!(stream.next().await.is_none(), "truncated at {len}");
  }
}

#[tokio::test]
async fn split_invalid_prefixes_are_rejected_without_reading_payloads() {
  for prefix in [
    Bytes::from_static(&[0, 0, 0x40, 0, 1]),
    Bytes::from_static(&[1, 0, 0, 0, 5]),
  ] {
    for split in 1..5 {
      let chunks = [prefix.slice(..split), prefix.slice(split..)];
      let chunks = futures_util::stream::iter(chunks.map(Ok::<_, std::io::Error>))
        .chain(futures_util::stream::pending());
      let mut stream = FrameDecoder::<Payload>::new(TakoBody::from_stream(chunks));
      let result = stream
        .next()
        .now_or_never()
        .expect("a rejected prefix must not wait for its payload");

      match prefix[0] {
        0 => assert!(matches!(result, Some(Err(GrpcError::MessageTooLarge)))),
        1 => assert!(matches!(
          result,
          Some(Err(GrpcError::CompressionUnsupported))
        )),
        _ => unreachable!(),
      }
      assert!(stream.next().await.is_none());
    }
  }
}

#[tokio::test]
async fn malformed_protobuf_ends_before_later_frames() {
  let mut chunk = vec![0, 0, 0, 0, 1, 0x0a];
  chunk.extend_from_slice(PAYLOAD_FRAME);
  let mut stream = decoder(vec![Bytes::from(chunk)]);

  assert!(matches!(
    stream.next().await,
    Some(Err(GrpcError::DecodeError(_)))
  ));
  assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn body_errors_end_an_incomplete_frame_once() {
  let frames = [
    Ok(Bytes::from_static(&PAYLOAD_FRAME[..7])),
    Err(std::io::Error::other("body read failed")),
    Ok(Bytes::from_static(PAYLOAD_FRAME)),
  ];
  let mut stream =
    FrameDecoder::<Payload>::new(TakoBody::from_stream(futures_util::stream::iter(frames)));

  assert!(matches!(
    stream.next().await,
    Some(Err(GrpcError::BodyReadError(_)))
  ));
  assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn trailers_do_not_interrupt_a_fragmented_message() {
  let frames = [
    Frame::data(Bytes::from_static(&PAYLOAD_FRAME[..3])),
    Frame::trailers(http::HeaderMap::new()),
    Frame::data(Bytes::from_static(&PAYLOAD_FRAME[3..])),
  ];
  let body = TakoBody::from_try_stream(futures_util::stream::iter(
    frames.map(Ok::<_, std::io::Error>),
  ));
  let mut stream = FrameDecoder::<Payload>::new(body);

  assert_eq!(stream.next().await.unwrap().unwrap().data.as_ref(), b"foo");
  assert!(stream.next().await.is_none());
}
