#![cfg(all(feature = "per-thread", feature = "ws"))]

use std::time::Duration;

use tako::PerThreadConfig;
use tako::router::Router;
use tako::spawn_per_thread;
use tako::types::Request;
use tako::ws::WebSocketUpgrade;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn worker_shutdown_sends_websocket_going_away_before_closing() {
  let mut router = Router::new();
  router.get("/ws", |request: Request| async move {
    WebSocketUpgrade::new(request, |mut websocket| async move {
      while let Some(Ok(_)) = websocket.recv().await {}
    })
  });
  let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
  let address = reservation.local_addr().unwrap();
  drop(reservation);
  let (workers, shutdown) = spawn_per_thread(
    &address.to_string(),
    router,
    PerThreadConfig {
      workers: 1,
      drain_timeout: Duration::from_millis(100),
      ..PerThreadConfig::default()
    },
  )
  .unwrap();
  shutdown.wait_for_bind_outcome(1).await.unwrap();

  let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
  client.write_all(b"GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n").await.unwrap();
  let mut head = Vec::new();
  while !head.ends_with(b"\r\n\r\n") {
    head.push(client.read_u8().await.unwrap());
  }
  assert!(head.starts_with(b"HTTP/1.1 101"));

  shutdown.trigger();
  let mut frames = Vec::new();
  let closed = tokio::time::timeout(Duration::from_secs(2), client.read_to_end(&mut frames)).await;
  for worker in workers {
    worker.join().unwrap();
  }
  closed.unwrap().unwrap();
  assert_eq!(frames, [0x88, 2, 0x03, 0xE9]);
}
