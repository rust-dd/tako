#![cfg(feature = "http3")]

#[path = "support/response_headers.rs"]
mod headers;
#[path = "support/h3.rs"]
mod support;

use bytes::Buf;

#[test]
fn h3_suppresses_connection_fields_in_headers_and_trailers() {
  support::run(headers::router(), |mut client| async move {
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
    let response = stream.recv_response().await.unwrap();
    headers::assert_clean(response.headers());
    let mut data = stream.recv_data().await.unwrap().unwrap();
    assert_eq!(data.copy_to_bytes(data.remaining()), "event");
    assert!(stream.recv_data().await.unwrap().is_none());
    headers::assert_clean(&stream.recv_trailers().await.unwrap().unwrap());
    drop(stream);
    let mut sse = client
      .sender
      .send_request(
        http::Request::builder()
          .uri("https://localhost/sse")
          .body(())
          .unwrap(),
      )
      .await
      .unwrap();
    sse.finish().await.unwrap();
    let response = sse.recv_response().await.unwrap();
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    assert!(!response.headers().contains_key("connection"));
    let mut event = sse.recv_data().await.unwrap().unwrap();
    assert_eq!(event.copy_to_bytes(event.remaining()), "data: event\n\n");
    assert!(sse.recv_data().await.unwrap().is_none());
    drop(sse);
    client.close().await;
  });
}
