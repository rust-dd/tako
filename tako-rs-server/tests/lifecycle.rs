#![cfg(not(feature = "compio"))]

use std::sync::Arc;
use std::time::Duration;

use http_body_util::BodyExt;
use hyper_util::rt::TokioIo;
use tako_rs_core::body::TakoBody;
use tako_rs_core::router::Router;
use tako_rs_server::Server;
use tako_rs_server::ServerConfig;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;

#[tokio::test]
async fn idle_http1_shutdown_finishes_promptly_and_releases_router_state() {
  let state = Arc::new(());
  let mut router = Router::new();
  router.with_state(state.clone());
  router.get("/", || async { "ok" });
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let handle = Server::builder().build().spawn_http(listener, router);
  assert_eq!(handle.local_addr(), Some(address));
  assert!(
    tokio::time::timeout(Duration::from_millis(5), handle.result())
      .await
      .is_err()
  );
  let (mut client, connection) =
    hyper::client::conn::http1::handshake(TokioIo::new(TcpStream::connect(address).await.unwrap()))
      .await
      .unwrap();
  let connection = tokio::spawn(connection);
  client
    .send_request(http::Request::new(TakoBody::empty()))
    .await
    .unwrap()
    .into_body()
    .collect()
    .await
    .unwrap();
  tokio::time::timeout(
    Duration::from_secs(1),
    handle.shutdown(Duration::from_secs(5)),
  )
  .await
  .expect("idle connections must not consume the drain budget");
  connection.await.unwrap().unwrap();
  assert_eq!(Arc::strong_count(&state), 1);
}

#[cfg(feature = "tls")]
#[tokio::test]
async fn missing_tls_and_bind_errors_are_returned_without_panicking() {
  let server = Server::builder().build();
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let handle = server.spawn_tls(listener, Router::new());
  assert!(
    handle
      .result()
      .await
      .unwrap_err()
      .to_string()
      .contains("certificate")
  );
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  assert!(server.try_spawn_tls(listener, Router::new()).is_err());
  let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let result = server.try_spawn_tcp_raw(occupied.local_addr().unwrap().to_string(), |_, _| {
    Box::pin(async { Ok(()) })
  });
  assert!(result.is_err());
}

#[tokio::test]
async fn completion_reports_success_after_the_listener_stops() {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let handle = Server::builder()
    .build()
    .try_spawn_http(listener, Router::new())
    .unwrap();
  handle.trigger();
  tokio::time::timeout(Duration::from_secs(1), handle.result())
    .await
    .unwrap()
    .unwrap();
  handle.result().await.unwrap();
}

#[tokio::test]
async fn explicit_shutdown_budget_cancels_a_hung_handler() {
  let started = Arc::new(tokio::sync::Notify::new());
  let marker = Arc::new(());
  let mut router = Router::new();
  let handler_marker = marker.clone();
  let handler_started = started.clone();
  router.get("/", move || {
    let marker = handler_marker.clone();
    let started = handler_started.clone();
    async move {
      started.notify_one();
      let response = std::future::pending::<&'static str>().await;
      drop(marker);
      response
    }
  });
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let handle = Server::builder().build().spawn_http(listener, router);
  let mut stream = TcpStream::connect(address).await.unwrap();
  stream
    .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
    .await
    .unwrap();
  tokio::time::timeout(Duration::from_secs(1), started.notified())
    .await
    .unwrap();
  tokio::time::timeout(
    Duration::from_millis(500),
    handle.shutdown(Duration::from_millis(20)),
  )
  .await
  .expect("shutdown must honor its shorter argument");
  let mut body = Vec::new();
  tokio::time::timeout(Duration::from_secs(1), stream.read_to_end(&mut body))
    .await
    .unwrap()
    .unwrap();
  assert_eq!(Arc::strong_count(&marker), 1);
}

#[tokio::test]
async fn incomplete_headers_observe_the_configured_deadline() {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let handle = Server::builder()
    .config(ServerConfig {
      header_read_timeout: Some(Duration::from_millis(25)),
      ..ServerConfig::default()
    })
    .build()
    .spawn_http(listener, Router::new());
  let mut stream = TcpStream::connect(address).await.unwrap();
  stream.write_all(b"GET / HTTP/1.1\r\nHost:").await.unwrap();
  let mut response = Vec::new();
  tokio::time::timeout(Duration::from_secs(1), stream.read_to_end(&mut response))
    .await
    .unwrap()
    .unwrap();
  handle.shutdown(Duration::from_secs(1)).await;
}

#[cfg(feature = "http2")]
#[tokio::test]
async fn idle_h2_shutdown_sends_goaway_without_waiting_for_the_drain_deadline() {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let mut router = Router::new();
  router.get("/", || async { "ok" });
  let handle = Server::builder().build().spawn_h2c(listener, router);
  let (mut client, connection) = hyper::client::conn::http2::handshake(
    hyper_util::rt::TokioExecutor::new(),
    TokioIo::new(TcpStream::connect(address).await.unwrap()),
  )
  .await
  .unwrap();
  let connection = tokio::spawn(connection);
  client
    .send_request(
      http::Request::builder()
        .uri("http://localhost/")
        .body(TakoBody::empty())
        .unwrap(),
    )
    .await
    .unwrap()
    .into_body()
    .collect()
    .await
    .unwrap();
  tokio::time::timeout(
    Duration::from_secs(1),
    handle.shutdown(Duration::from_secs(5)),
  )
  .await
  .unwrap();
  tokio::time::timeout(Duration::from_secs(1), connection)
    .await
    .unwrap()
    .unwrap()
    .unwrap();
  assert!(client.ready().await.is_err());
}
