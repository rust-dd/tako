#![cfg(feature = "http3")]

#[path = "support/h3.rs"]
mod support;

use bytes::Buf;
use bytes::Bytes;
use futures_util::StreamExt;
use tako_rs_core::body::TakoBody;
use tako_rs_core::router::Router;
use tokio_util::sync::CancellationToken;

#[test]
fn response_body_error_resets_the_stream_after_the_prefix() {
  let fail = CancellationToken::new();
  let handler_fail = fail.clone();
  let mut router = Router::new();
  router.get("/healthy", || async { "complete" });
  router.get("/", move || {
    let fail = handler_fail.clone();
    async move {
      let prefix = futures_util::stream::once(async {
        Ok::<_, std::io::Error>(Bytes::from_static(b"prefix"))
      });
      let error = futures_util::stream::once(async move {
        fail.cancelled().await;
        Err(std::io::Error::other("source failed"))
      });
      TakoBody::from_stream(prefix.chain(error))
    }
  });
  support::run(router, move |mut client| async move {
    let mut stream = client
      .sender
      .send_request(
        http::Request::builder()
          .uri("https://localhost/")
          .body(())
          .unwrap(),
      )
      .await
      .unwrap();
    stream.finish().await.unwrap();
    assert_eq!(stream.recv_response().await.unwrap().status(), 200);
    let mut prefix = stream.recv_data().await.unwrap().unwrap();
    assert_eq!(prefix.copy_to_bytes(prefix.remaining()), "prefix");
    fail.cancel();
    let outcome = stream.recv_data().await;
    assert!(
      matches!(
        outcome,
        Err(h3::error::StreamError::RemoteTerminate {
          code: h3::error::Code::H3_INTERNAL_ERROR,
          ..
        })
      ),
      "a failed source must reset the response with H3_INTERNAL_ERROR"
    );
    drop(stream);
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
    healthy.finish().await.unwrap();
    assert_eq!(healthy.recv_response().await.unwrap().status(), 200);
    let mut data = healthy.recv_data().await.unwrap().unwrap();
    assert_eq!(data.copy_to_bytes(data.remaining()), "complete");
    assert!(healthy.recv_data().await.unwrap().is_none());
    drop(healthy);
    client.close().await;
  });
}
