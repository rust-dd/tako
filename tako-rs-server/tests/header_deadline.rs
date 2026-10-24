#![cfg(not(feature = "compio"))]

use std::time::Duration;

use bytes::Bytes;
use tako_rs_core::body::TakoBody;
use tako_rs_core::router::Router;
use tako_rs_server::Server;
use tako_rs_server::ServerConfig;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn header_deadline_must_not_truncate_a_buffered_response() {
  exercise_backpressure(false).await;
}

#[tokio::test]
async fn header_deadline_must_not_truncate_a_streamed_response() {
  exercise_backpressure(true).await;
}

async fn exercise_backpressure(streamed: bool) {
  const SIZE: usize = 16 * 1024 * 1024;
  let mut router = Router::new();
  router.get("/", move || async move {
    let data = Bytes::from(vec![b'x'; SIZE]);
    let body = if streamed {
      TakoBody::from_stream(futures_util::stream::once(async {
        Ok::<_, std::io::Error>(data)
      }))
    } else {
      TakoBody::from(data)
    };
    http::Response::builder()
      .header("content-length", SIZE)
      .body(body)
      .unwrap()
  });
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let server = Server::builder()
    .config(ServerConfig {
      header_read_timeout: Some(Duration::from_millis(20)),
      ..ServerConfig::default()
    })
    .build()
    .try_spawn_http(listener, router)
    .unwrap();
  let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
  client
    .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
    .await
    .unwrap();
  let mut head = Vec::new();
  while !head.ends_with(b"\r\n\r\n") {
    let mut byte = [0];
    client.read_exact(&mut byte).await.unwrap();
    head.push(byte[0]);
  }
  assert!(head.starts_with(b"HTTP/1.1 200"));
  tokio::time::sleep(Duration::from_millis(100)).await;
  let mut body = Vec::new();
  let result = tokio::time::timeout(Duration::from_secs(3), client.read_to_end(&mut body))
    .await
    .unwrap();
  server.shutdown(Duration::from_secs(1)).await;
  assert_eq!(
    body.len(),
    SIZE,
    "header deadline truncated a response: read result {result:?}"
  );
}
