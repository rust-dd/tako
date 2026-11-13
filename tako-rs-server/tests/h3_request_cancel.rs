#![cfg(feature = "http3")]

#[path = "support/h3.rs"]
mod support;

use std::pin::Pin;
use std::task::Poll;

use bytes::Buf;
use http_body::Body;
use http_body_util::BodyExt;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;

#[test]
fn unread_upload_stops_without_invalidating_the_response() {
  let mut router = Router::new();
  router.post("/", || async { "ok" });
  check_early_response(router, false);
}

#[test]
fn cancelled_body_read_stops_the_upload_without_waiting_for_more_data() {
  let mut router = Router::new();
  router.post("/", |mut request: Request| async move {
    std::future::poll_fn(|cx| {
      assert!(Pin::new(request.body_mut()).poll_frame(cx).is_pending());
      Poll::Ready(())
    })
    .await;
    "ok"
  });
  check_early_response(router, false);
}

#[test]
fn partially_read_upload_stops_without_invalidating_the_response() {
  let mut router = Router::new();
  router.post("/", |mut request: Request| async move {
    let frame = request.body_mut().frame().await.unwrap().unwrap();
    assert_eq!(frame.into_data().unwrap(), "x");
    "ok"
  });
  check_early_response(router, true);
}

fn check_early_response(mut router: Router, send_data: bool) {
  router.get("/healthy", || async { "healthy" });
  support::run(router, move |mut client| async move {
    let (mut upload, mut response) = client.connection.open_bi().await.unwrap();
    // Static QPACK fields encode POST https://localhost/; keeping the send half open tests
    // cancellation without letting a client library normalize the peer's STOP_SENDING code.
    upload
      .write_all(b"\x01\x10\x00\x00\xd4\xd7\x50\x09localhost\xc1")
      .await
      .unwrap();
    if send_data {
      upload.write_all(b"\x00\x01x").await.unwrap();
    }
    let stopped = upload.stopped().await.unwrap().unwrap();
    assert_eq!(
      stopped.into_inner(),
      0x100,
      "early response must use H3_NO_ERROR"
    );
    let bytes = response.read_to_end(4096).await.unwrap();
    let mut frames = bytes.as_slice();
    let mut body = Vec::new();
    while !frames.is_empty() {
      let kind = read_varint(&mut frames);
      let length = usize::try_from(read_varint(&mut frames)).unwrap();
      let (payload, rest) = frames.split_at(length);
      if kind == 0 {
        body.extend_from_slice(payload);
      }
      frames = rest;
    }
    assert_eq!(body, b"ok");

    let mut healthy = client
      .sender
      .send_request(
        http::Request::builder()
          .uri("https://localhost/healthy")
          .body(())
          .unwrap(),
      )
      .await
      .unwrap();
    if let Err(error) = healthy.finish().await {
      assert!(matches!(
        error,
        h3::error::StreamError::RemoteTerminate {
          code: h3::error::Code::H3_NO_ERROR,
          ..
        }
      ));
    }
    assert_eq!(healthy.recv_response().await.unwrap().status(), 200);
    while healthy.recv_data().await.unwrap().is_some() {}
    drop(healthy);
    client.close().await;
  });
}

fn read_varint(bytes: &mut &[u8]) -> u64 {
  let first = bytes.get_u8();
  let width = 1 << (first >> 6);
  (1..width).fold(u64::from(first & 0x3f), |value, _| {
    (value << 8) | u64::from(bytes.get_u8())
  })
}
