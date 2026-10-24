#![cfg(feature = "http2")]

#[path = "support/response_headers.rs"]
mod headers;

use std::net::SocketAddr;
use std::time::Duration;

use http_body_util::BodyExt;
use tako_rs_core::body::TakoBody;

async fn client(address: SocketAddr) {
  let stream = tokio::net::TcpStream::connect(address).await.unwrap();
  let (mut sender, connection) = hyper::client::conn::http2::handshake(
    hyper_util::rt::TokioExecutor::new(),
    hyper_util::rt::TokioIo::new(stream),
  )
  .await
  .unwrap();
  let connection = tokio::spawn(connection);
  let response = sender
    .send_request(
      http::Request::builder()
        .uri("http://localhost/")
        .body(TakoBody::empty())
        .unwrap(),
    )
    .await
    .unwrap();
  headers::assert_clean(response.headers());
  let body = response.into_body().collect().await.unwrap();
  headers::assert_clean(body.trailers().unwrap());
  assert_eq!(body.to_bytes(), "event");
  let response = sender
    .send_request(
      http::Request::builder()
        .uri("http://localhost/sse")
        .body(TakoBody::empty())
        .unwrap(),
    )
    .await
    .unwrap();
  assert_eq!(response.headers()["content-type"], "text/event-stream");
  assert!(!response.headers().contains_key("connection"));
  assert_eq!(
    response.into_body().collect().await.unwrap().to_bytes(),
    "data: event\n\n"
  );
  drop(sender);
  connection.await.unwrap().unwrap();
}

#[cfg(not(feature = "compio"))]
#[tokio::test]
async fn h2_suppresses_connection_fields_in_headers_and_trailers() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let server = tako_rs_server::Server::builder()
    .build()
    .spawn_h2c(listener, headers::router());
  tokio::time::timeout(Duration::from_secs(5), client(address))
    .await
    .unwrap();
  server.shutdown(Duration::from_secs(1)).await;
}

#[cfg(feature = "compio")]
#[test]
fn h2_suppresses_connection_fields_in_headers_and_trailers() {
  compio::runtime::Runtime::new().unwrap().block_on(async {
    let listener = compio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tako_rs_server::CompioServer::builder()
      .build()
      .spawn_h2c(listener, headers::router());
    let (done, finished) = tokio::sync::oneshot::channel();
    let thread = std::thread::spawn(move || {
      tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(client(address));
      let _ = done.send(());
    });
    let result = compio::time::timeout(Duration::from_secs(5), finished).await;
    server.shutdown(Duration::from_secs(1)).await;
    thread.join().unwrap();
    result.unwrap().unwrap();
  });
}
