//! End-to-end tests for the unified `Server::builder()` entry point.
//!
//! These spin up a real TCP listener, hit it with a tokio TcpStream, and
//! verify the response. The compio path is excluded — these tests only run on
//! the default tokio transport.

#![cfg(not(feature = "compio"))]

use std::time::Duration;

use http::{Method, StatusCode};
use tako::body::TakoBody;
use tako::router::Router;
use tako::types::Request;
use tako::{Server, ServerConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

async fn hello(_req: Request) -> &'static str {
  "ok"
}

async fn fetch_status_line(addr: &std::net::SocketAddr) -> String {
  let mut stream = TcpStream::connect(addr).await.unwrap();
  let _ = stream
    .write_all(b"GET /ping HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
    .await;
  let mut buf = Vec::new();
  let _ = stream.read_to_end(&mut buf).await;
  let txt = String::from_utf8_lossy(&buf).to_string();
  txt.lines().next().unwrap_or("").to_string()
}

#[tokio::test]
async fn server_builder_handles_http_request() {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();

  let mut router = Router::new();
  router.get("/ping", hello);

  let server = Server::builder().config(ServerConfig::default()).build();
  let handle = server.spawn_http(listener, router);

  // Tiny delay to let the spawn task start the accept loop.
  tokio::time::sleep(Duration::from_millis(50)).await;

  let status_line = fetch_status_line(&addr).await;
  assert!(
    status_line.starts_with("HTTP/1.1 200"),
    "status line was {status_line:?}"
  );

  // Trigger graceful shutdown.
  handle.shutdown(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn server_builder_default_config_is_default() {
  let server = Server::builder().build();
  assert_eq!(server.config().drain_timeout, Duration::from_secs(30));
  assert_eq!(server.config().h2_max_concurrent_streams, 100);
}

// Smoke-test the raw TCP path on the builder.
#[tokio::test]
async fn server_builder_handles_raw_tcp_echo() {
  use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

  // Bind once just to discover a free port, then drop the listener so the
  // builder can rebind via serve_tcp_with_shutdown.
  let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = probe.local_addr().unwrap();
  drop(probe);

  let server = Server::builder().build();
  let handle = server.spawn_tcp_raw(addr.to_string(), |mut stream, _peer| {
    Box::pin(async move {
      let mut buf = [0u8; 16];
      let n = stream.read(&mut buf).await?;
      stream.write_all(&buf[..n]).await?;
      Ok(())
    })
  });

  tokio::time::sleep(Duration::from_millis(80)).await;

  let mut s = TcpStream::connect(&addr).await.unwrap();
  s.write_all(b"hi").await.unwrap();
  let mut out = [0u8; 16];
  let n = s.read(&mut out).await.unwrap();
  assert_eq!(&out[..n], b"hi");

  handle.shutdown(Duration::from_secs(2)).await;
}

// Smoke-test the raw UDP path on the builder.
#[tokio::test]
async fn server_builder_handles_raw_udp_echo() {
  let probe = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
  let addr = probe.local_addr().unwrap();
  drop(probe);

  let server = Server::builder().build();
  let handle = server.spawn_udp_raw(addr.to_string(), |data, peer, sock| {
    Box::pin(async move {
      let _ = sock.send_to(&data, peer).await;
    })
  });

  tokio::time::sleep(Duration::from_millis(80)).await;

  let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
  client.send_to(b"udp-hi", &addr).await.unwrap();
  let mut buf = [0u8; 32];
  let recv = tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buf))
    .await
    .unwrap()
    .unwrap();
  assert_eq!(&buf[..recv.0], b"udp-hi");

  handle.trigger();
}

// Smoke-test that the 405 + Allow path keeps working through the builder
// without any caller-side wiring.
#[tokio::test]
async fn server_builder_propagates_405_with_allow() {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();

  let mut router = Router::new();
  router.route(Method::GET, "/only-get", hello);

  let server = Server::builder().build();
  let handle = server.spawn_http(listener, router);

  tokio::time::sleep(Duration::from_millis(50)).await;

  let mut stream = TcpStream::connect(&addr).await.unwrap();
  let _ = stream
    .write_all(
      b"POST /only-get HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    )
    .await;
  let mut buf = Vec::new();
  let _ = stream.read_to_end(&mut buf).await;
  let txt = String::from_utf8_lossy(&buf);
  assert!(
    txt.contains(&format!("HTTP/1.1 {}", StatusCode::METHOD_NOT_ALLOWED.as_u16())),
    "wanted 405, got: {txt:?}"
  );
  assert!(
    txt.to_ascii_lowercase().contains("allow:"),
    "wanted Allow header, got: {txt:?}"
  );

  // Drop the in-flight body so we can use TakoBody import without a warning.
  let _ = TakoBody::empty();

  handle.shutdown(Duration::from_secs(2)).await;
}
