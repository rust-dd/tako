#![cfg(all(feature = "ws", not(feature = "compio")))]

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::SinkExt;
use futures_util::StreamExt;
use tako::Server;
use tako::ServerConfig;
use tako::router::Router;
use tako::types::Request;
use tako::ws::TakoWs;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::time::timeout;

const UPGRADE: &[u8] = b"GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
Sec-WebSocket-Version: 13\r\n\r\n";

fn router() -> Router {
  let mut router = Router::new();
  router.get("/", || async { "ok" });
  router.get("/ws", |req: Request| async move {
    TakoWs::new(req, |mut ws| async move {
      let worker = std::thread::current()
        .name()
        .unwrap_or_default()
        .to_string();
      if ws.send(worker.into()).await.is_ok() {
        while let Some(Ok(_)) = ws.next().await {}
      }
    })
  });
  router
}

async fn read_some(stream: &mut TcpStream, into: &mut Vec<u8>) {
  let mut chunk = [0; 512];
  let read = timeout(Duration::from_secs(2), stream.read(&mut chunk))
    .await
    .unwrap()
    .unwrap();
  assert_ne!(read, 0, "connection closed early");
  into.extend_from_slice(&chunk[..read]);
}

/// Upgrades a connection and returns it with the worker named in the first frame.
async fn open_websocket(address: SocketAddr) -> (TcpStream, String) {
  let mut stream = TcpStream::connect(address).await.unwrap();
  stream.write_all(UPGRADE).await.unwrap();
  let mut received = Vec::new();
  let head_end = loop {
    read_some(&mut stream, &mut received).await;
    if let Some(end) = received.windows(4).position(|w| w == b"\r\n\r\n") {
      break end + 4;
    }
  };
  assert!(
    received.starts_with(b"HTTP/1.1 101"),
    "{}",
    String::from_utf8_lossy(&received)
  );
  let mut frame = received.split_off(head_end);
  while frame.len() < 2 || frame.len() < 2 + usize::from(frame[1]) {
    read_some(&mut stream, &mut frame).await;
  }
  assert_eq!(frame[0], 0x81, "expected an unfragmented text frame");
  let worker = String::from_utf8(frame[2..2 + usize::from(frame[1])].to_vec()).unwrap();
  (stream, worker)
}

async fn answers_within(stream: &mut TcpStream, wait: Duration) -> bool {
  let mut byte = [0; 1];
  matches!(timeout(wait, stream.read(&mut byte)).await, Ok(Ok(1)))
}

async fn assert_websocket_holds_the_only_slot(address: SocketAddr) {
  let (websocket, _) = open_websocket(address).await;
  let mut second = TcpStream::connect(address).await.unwrap();
  second
    .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
    .await
    .unwrap();
  assert!(
    !answers_within(&mut second, Duration::from_millis(300)).await,
    "served while the WebSocket held the only connection slot"
  );
  drop(websocket);
  assert!(
    answers_within(&mut second, Duration::from_secs(2)).await,
    "not served after the WebSocket closed"
  );
}

#[tokio::test]
async fn websockets_count_against_max_connections() {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let handle = Server::builder()
    .config(ServerConfig {
      max_connections: Some(1),
      ..ServerConfig::default()
    })
    .build()
    .spawn_http(listener, router());
  assert_websocket_holds_the_only_slot(address).await;
  handle.shutdown(Duration::from_secs(1)).await;
}

#[cfg(all(unix, feature = "per-thread"))]
mod per_thread {
  use std::collections::HashMap;
  use std::net::SocketAddr;
  use std::time::Duration;

  use tako::PerThreadConfig;
  use tako::spawn_per_thread;
  use tokio::time::timeout;

  use super::assert_websocket_holds_the_only_slot;
  use super::open_websocket;
  use super::router;

  fn free_address() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
  }

  #[tokio::test]
  async fn websockets_count_against_max_connections() {
    let address = free_address();
    let (threads, shutdown) = spawn_per_thread(
      &address.to_string(),
      router(),
      PerThreadConfig {
        workers: 1,
        pin_to_core: false,
        max_connections: Some(1),
        ..PerThreadConfig::default()
      },
    )
    .unwrap();
    timeout(Duration::from_secs(2), shutdown.wait_for_bind_outcome(1))
      .await
      .unwrap()
      .unwrap();

    assert_websocket_holds_the_only_slot(address).await;

    shutdown.trigger();
    tokio::task::spawn_blocking(move || threads.into_iter().for_each(|t| t.join().unwrap()))
      .await
      .unwrap();
  }

  #[tokio::test]
  async fn websockets_are_spread_evenly_across_workers() {
    let address = free_address();
    let (threads, shutdown) = spawn_per_thread(
      &address.to_string(),
      router(),
      PerThreadConfig {
        workers: 4,
        pin_to_core: false,
        ..PerThreadConfig::default()
      },
    )
    .unwrap();
    timeout(Duration::from_secs(2), shutdown.wait_for_bind_outcome(4))
      .await
      .unwrap()
      .unwrap();

    let mut open = Vec::new();
    let mut per_worker = HashMap::<String, usize>::new();
    for _ in 0..8 {
      let (stream, worker) = open_websocket(address).await;
      *per_worker.entry(worker).or_default() += 1;
      open.push(stream);
    }
    assert_eq!(per_worker.len(), 4, "{per_worker:?}");
    assert!(
      per_worker.values().all(|&count| count == 2),
      "{per_worker:?}"
    );

    drop(open);
    shutdown.trigger();
    tokio::task::spawn_blocking(move || threads.into_iter().for_each(|t| t.join().unwrap()))
      .await
      .unwrap();
  }
}
