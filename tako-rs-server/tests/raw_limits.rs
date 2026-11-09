#![cfg(not(feature = "compio"))]

use std::sync::Arc;
use std::time::Duration;

use tako_rs_server::Server;
use tako_rs_server::ServerConfig;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

#[tokio::test]
async fn raw_tcp_waits_for_capacity_and_shutdown_cancels_the_active_handler() {
  let (started, mut received) = tokio::sync::mpsc::unbounded_channel();
  let release = Arc::new(tokio::sync::Semaphore::new(0));
  let handler_release = release.clone();
  let handle = Server::builder()
    .config(ServerConfig {
      max_connections: Some(1),
      drain_timeout: Duration::from_millis(20),
      ..ServerConfig::default()
    })
    .build()
    .try_spawn_tcp_raw("127.0.0.1:0", move |stream, _| {
      let started = started.clone();
      let release = handler_release.clone();
      Box::pin(async move {
        started.send(()).unwrap();
        release.acquire().await.unwrap().forget();
        drop(stream);
        Ok(())
      })
    })
    .unwrap();
  let address = handle.local_addr().unwrap();
  let _first = TcpStream::connect(address).await.unwrap();
  tokio::time::timeout(Duration::from_secs(1), received.recv())
    .await
    .unwrap()
    .unwrap();
  let mut second = TcpStream::connect(address).await.unwrap();
  assert!(
    tokio::time::timeout(Duration::from_millis(75), received.recv())
      .await
      .is_err(),
    "a second raw TCP handler started while max_connections was exhausted"
  );
  release.add_permits(1);
  tokio::time::timeout(Duration::from_secs(1), received.recv())
    .await
    .unwrap()
    .unwrap();
  handle.trigger();
  tokio::time::timeout(Duration::from_secs(1), handle.result())
    .await
    .unwrap()
    .unwrap();
  assert_eq!(second.read(&mut [0]).await.unwrap(), 0);
}

#[cfg(feature = "udp")]
#[tokio::test]
async fn raw_udp_drops_excess_datagrams_and_resumes_after_capacity_returns() {
  let (started, mut received) = tokio::sync::mpsc::unbounded_channel();
  let release = Arc::new(tokio::sync::Semaphore::new(0));
  let handler_release = release.clone();
  let handle = Server::builder()
    .config(ServerConfig {
      max_connections: Some(1),
      drain_timeout: Duration::from_millis(20),
      ..ServerConfig::default()
    })
    .build()
    .try_spawn_udp_raw("127.0.0.1:0", move |data, _, _| {
      let started = started.clone();
      let release = handler_release.clone();
      Box::pin(async move {
        started.send(data).unwrap();
        release.acquire().await.unwrap().forget();
      })
    })
    .unwrap();
  let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
  let address = handle.local_addr().unwrap();
  socket.send_to(b"first", address).await.unwrap();
  assert_eq!(
    tokio::time::timeout(Duration::from_secs(1), received.recv())
      .await
      .unwrap()
      .unwrap(),
    b"first"
  );
  for _ in 0..32 {
    socket.send_to(b"excess", address).await.unwrap();
  }
  assert!(
    tokio::time::timeout(Duration::from_millis(75), received.recv())
      .await
      .is_err(),
    "UDP started excess handlers instead of dropping datagrams"
  );
  release.add_permits(1);
  tokio::time::sleep(Duration::from_millis(20)).await;
  socket.send_to(b"after", address).await.unwrap();
  assert_eq!(
    tokio::time::timeout(Duration::from_secs(1), received.recv())
      .await
      .unwrap()
      .unwrap(),
    b"after",
    "datagrams received at capacity must not be queued for later handlers"
  );
  handle.trigger();
  tokio::time::timeout(Duration::from_secs(1), handle.result())
    .await
    .unwrap()
    .unwrap();
}
