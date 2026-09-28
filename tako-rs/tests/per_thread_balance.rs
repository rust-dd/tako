#![cfg(all(unix, feature = "per-thread", not(feature = "compio")))]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use tako::PerThreadConfig;
use tako::router::Router;
use tako::spawn_per_thread;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

fn free_address() -> SocketAddr {
  let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
  listener.local_addr().unwrap()
}

async fn served_by(stream: &mut TcpStream) -> String {
  stream
    .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
    .await
    .unwrap();
  let mut response = Vec::new();
  let mut chunk = [0; 1024];
  loop {
    let read = tokio::time::timeout(Duration::from_secs(1), stream.read(&mut chunk))
      .await
      .unwrap()
      .unwrap();
    assert_ne!(read, 0, "connection closed before the response completed");
    response.extend_from_slice(&chunk[..read]);
    let text = String::from_utf8_lossy(&response);
    if let Some((head, body)) = text.split_once("\r\n\r\n") {
      let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap();
      if body.len() >= length {
        return body[..length].to_string();
      }
    }
  }
}

#[tokio::test]
async fn keep_alive_connections_are_spread_evenly_across_workers() {
  let mut router = Router::new();
  router.get("/", || async {
    std::thread::current()
      .name()
      .unwrap_or_default()
      .to_string()
  });
  let address = free_address();
  let (threads, shutdown) = spawn_per_thread(
    &address.to_string(),
    router,
    PerThreadConfig {
      workers: 4,
      pin_to_core: false,
      ..PerThreadConfig::default()
    },
  )
  .unwrap();
  tokio::time::timeout(Duration::from_secs(2), shutdown.wait_for_bind_outcome(4))
    .await
    .unwrap()
    .unwrap();

  let mut open = Vec::new();
  let mut per_worker = HashMap::<String, usize>::new();
  for _ in 0..8 {
    let mut stream = TcpStream::connect(address).await.unwrap();
    *per_worker.entry(served_by(&mut stream).await).or_default() += 1;
    open.push(stream);
  }
  assert_eq!(per_worker.len(), 4, "{per_worker:?}");
  assert!(
    per_worker.values().all(|&count| count == 2),
    "{per_worker:?}"
  );

  drop(open);
  shutdown.trigger();
  tokio::task::spawn_blocking(move || {
    for thread in threads {
      thread.join().unwrap();
    }
  })
  .await
  .unwrap();
}
