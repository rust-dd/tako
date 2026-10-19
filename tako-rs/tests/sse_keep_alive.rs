#![cfg(feature = "sse")]

use std::time::Duration;

use bytes::Bytes;
use futures_util::stream;
use http_body_util::BodyExt;
use tako::responder::Responder;
use tako::sse::Sse;
use tako::sse::SseEvent;

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn idle_raw_stream_emits_repeated_keep_alive_frames() {
  let mut body = Sse::new(stream::pending::<Bytes>())
    .keep_alive(Duration::from_millis(1))
    .into_response()
    .into_body();

  for _ in 0..2 {
    let frame = body.frame().await.unwrap().unwrap().into_data().unwrap();
    assert_eq!(frame, ":keepalive\n\n");
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn idle_event_stream_emits_keep_alive_frames() {
  let mut body = Sse::events(stream::pending::<SseEvent>())
    .keep_alive(Duration::from_millis(1))
    .into_response()
    .into_body();

  let frame = body.frame().await.unwrap().unwrap().into_data().unwrap();
  assert_eq!(frame, ":keepalive\n\n");
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn completed_event_stream_stops_keep_alive_frames() {
  let mut body = Sse::events(stream::iter([SseEvent::data("finished")]))
    .keep_alive(Duration::from_millis(1))
    .into_response()
    .into_body();

  let frame = body.frame().await.unwrap().unwrap().into_data().unwrap();
  assert_eq!(frame, "data: finished\n\n");
  assert!(body.frame().await.is_none());
}

#[cfg(not(feature = "compio"))]
#[tokio::test(start_paused = true)]
async fn event_and_keep_alive_frames_restart_the_idle_period() {
  use futures_util::future::poll_immediate;

  let (sender, receiver) = tokio::sync::mpsc::channel(1);
  let events = stream::unfold(receiver, |mut receiver| async {
    receiver.recv().await.map(|event| (event, receiver))
  });
  let mut body = Sse::new(events)
    .keep_alive(Duration::from_secs(10))
    .into_response()
    .into_body();

  assert!(poll_immediate(body.frame()).await.is_none());
  tokio::time::advance(Duration::from_secs(5)).await;
  sender.send(Bytes::from_static(b"payload")).await.unwrap();
  let frame = body.frame().await.unwrap().unwrap().into_data().unwrap();
  assert_eq!(frame, "data: payload\n\n");

  tokio::time::advance(Duration::from_secs(5)).await;
  assert!(poll_immediate(body.frame()).await.is_none());
  tokio::time::advance(Duration::from_secs(5)).await;
  let frame = body.frame().await.unwrap().unwrap().into_data().unwrap();
  assert_eq!(frame, ":keepalive\n\n");

  tokio::time::advance(Duration::from_secs(9)).await;
  assert!(poll_immediate(body.frame()).await.is_none());
  tokio::time::advance(Duration::from_secs(1)).await;
  let frame = body.frame().await.unwrap().unwrap().into_data().unwrap();
  assert_eq!(frame, ":keepalive\n\n");
}
