#![cfg(not(feature = "compio"))]

use std::time::Duration;

use tako_rs_core::router::Router;
use tako_rs_server_pt::PerThreadConfig;
use tako_rs_server_pt::spawn_per_thread;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn header_timeout_waits_for_the_complete_response_and_then_closes_idle_connection() {
  const SIZE: usize = 16 * 1024 * 1024;
  let mut router = Router::new();
  router.get("/", || async { "x".repeat(SIZE) });
  let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
  let address = reservation.local_addr().unwrap();
  drop(reservation);
  let (workers, shutdown) = spawn_per_thread(
    &address.to_string(),
    router,
    PerThreadConfig {
      workers: 1,
      header_read_timeout: Some(Duration::from_millis(20)),
      drain_timeout: Duration::from_millis(100),
      ..PerThreadConfig::default()
    },
  )
  .unwrap();
  shutdown.wait_for_bind_outcome(1).await.unwrap();
  let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
  client
    .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
    .await
    .unwrap();
  let mut head = Vec::new();
  while !head.ends_with(b"\r\n\r\n") {
    head.push(client.read_u8().await.unwrap());
  }
  tokio::time::sleep(Duration::from_millis(100)).await;
  let mut body = Vec::new();
  let read = tokio::time::timeout(Duration::from_secs(3), client.read_to_end(&mut body)).await;
  shutdown.trigger();
  for worker in workers {
    worker.join().unwrap();
  }
  read.expect("idle connection must still close").unwrap();
  assert_eq!(body.len(), SIZE, "response was truncated before flushing");
}
