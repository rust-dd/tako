#![cfg(feature = "compio")]

use std::time::Duration;

use tako_rs_core::body::TakoBody;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;
use tako_rs_server_pt::PerThreadConfig;
use tako_rs_server_pt::spawn_per_thread;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn upgraded_connection_holds_its_worker_slot_until_io_is_dropped() {
  let release = CancellationToken::new();
  let handler_release = release.clone();
  let mut router = Router::new();
  router.get("/", || async { "ok" });
  router.get("/upgrade", move |mut req: Request| {
    let release = handler_release.clone();
    async move {
      let upgraded = hyper::upgrade::on(&mut req);
      compio::runtime::spawn(async move {
        let io = upgraded.await.unwrap();
        release.cancelled().await;
        drop(io);
      })
      .detach();
      hyper::Response::builder()
        .status(101)
        .header("connection", "upgrade")
        .header("upgrade", "test")
        .body(TakoBody::empty())
        .unwrap()
    }
  });
  let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
  let address = reservation.local_addr().unwrap();
  drop(reservation);
  let (workers, shutdown) = spawn_per_thread(
    &address.to_string(),
    router,
    PerThreadConfig {
      workers: 1,
      max_connections: Some(1),
      drain_timeout: Duration::from_millis(100),
      ..PerThreadConfig::default()
    },
  )
  .unwrap();
  shutdown.wait_for_bind_outcome(1).await.unwrap();
  let mut upgraded = tokio::net::TcpStream::connect(address).await.unwrap();
  upgraded
    .write_all(
      b"GET /upgrade HTTP/1.1\r\nHost: localhost\r\nConnection: upgrade\r\nUpgrade: test\r\n\r\n",
    )
    .await
    .unwrap();
  let mut head = Vec::new();
  while !head.ends_with(b"\r\n\r\n") {
    head.push(upgraded.read_u8().await.unwrap());
  }
  assert!(head.starts_with(b"HTTP/1.1 101"));
  let second = tokio::spawn(async move {
    let mut second = tokio::net::TcpStream::connect(address).await.unwrap();
    second
      .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
      .await
      .unwrap();
    assert_eq!(second.read_u8().await.unwrap(), b'H');
  });
  tokio::time::sleep(Duration::from_millis(150)).await;
  let served_early = second.is_finished();
  release.cancel();
  let resumed = tokio::time::timeout(Duration::from_secs(2), second).await;
  shutdown.trigger();
  for worker in workers {
    worker.join().unwrap();
  }
  resumed.unwrap().unwrap();
  assert!(
    !served_early,
    "upgrade released the only worker connection permit too early"
  );
}
