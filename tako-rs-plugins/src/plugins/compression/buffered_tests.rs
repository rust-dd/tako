use std::collections::VecDeque;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use bytes::Bytes;
use http::HeaderMap;
use http_body::Body;
use http_body::Frame;
use tako_rs_core::body::TakoBody;
use tako_rs_core::types::BoxError;

use super::frames::collect_buffered;

struct Frames(VecDeque<Result<Frame<Bytes>, BoxError>>);

impl Body for Frames {
  type Data = Bytes;
  type Error = BoxError;

  fn poll_frame(
    mut self: Pin<&mut Self>,
    _: &mut Context<'_>,
  ) -> Poll<Option<Result<Frame<Bytes>, BoxError>>> {
    Poll::Ready(self.0.pop_front())
  }

  fn is_end_stream(&self) -> bool {
    self.0.is_empty()
  }
}

fn trailers() -> HeaderMap {
  let mut headers = HeaderMap::new();
  headers.insert("x-final-state", "complete".parse().unwrap());
  headers
}

#[tokio::test]
async fn buffered_collection_waits_for_pending_data_and_trailers() {
  let mut frames = [
    Ok::<_, BoxError>(Frame::data(Bytes::from_static(b"first"))),
    Ok(Frame::data(Bytes::new())),
    Ok(Frame::data(Bytes::from_static(b" second"))),
    Ok(Frame::trailers(trailers())),
  ]
  .into_iter();
  let mut should_yield = false;
  let body = TakoBody::from_try_stream(futures_util::stream::poll_fn(move |cx| {
    if should_yield {
      should_yield = false;
      cx.waker().wake_by_ref();
      return Poll::Pending;
    }
    let frame = frames.next();
    should_yield = frame.is_some();
    Poll::Ready(frame)
  }));
  let mut collected = std::pin::pin!(collect_buffered(body));
  assert!(futures_util::poll!(collected.as_mut()).is_pending());
  let (data, received_trailers) = collected.await.unwrap();
  assert_eq!(data, "first second");
  assert_eq!(received_trailers, Some(trailers()));
}

#[tokio::test]
async fn buffered_collection_preserves_a_final_trailer_frame() {
  for frames in [
    VecDeque::from([Ok(Frame::trailers(trailers()))]),
    VecDeque::from([
      Ok(Frame::data(Bytes::new())),
      Ok(Frame::trailers(trailers())),
    ]),
  ] {
    let (data, received_trailers) = collect_buffered(TakoBody::new(Frames(frames)))
      .await
      .unwrap();
    assert!(data.is_empty());
    assert_eq!(received_trailers, Some(trailers()));
  }
}

#[tokio::test]
async fn buffered_collection_preserves_empty_bodies() {
  for body in [
    TakoBody::empty(),
    TakoBody::new(Frames(VecDeque::from([Ok(Frame::data(Bytes::new()))]))),
  ] {
    let (data, received_trailers) = collect_buffered(body).await.unwrap();
    assert!(data.is_empty());
    assert!(received_trailers.is_none());
  }
}

#[tokio::test]
async fn buffered_collection_propagates_an_error_after_data() {
  let body = TakoBody::new(Frames(VecDeque::from([
    Ok(Frame::data(Bytes::from_static(b"first"))),
    Err(std::io::Error::other("source failed").into()),
  ])));
  assert_eq!(
    collect_buffered(body).await.unwrap_err().to_string(),
    "source failed"
  );
}
