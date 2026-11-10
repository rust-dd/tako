#![cfg(all(unix, not(feature = "compio")))]

use std::sync::Arc;
use std::time::Duration;

use tako_rs_server::Server;
use tako_rs_server::ServerConfig;
use tokio::io::AsyncReadExt;

#[tokio::test]
async fn raw_unix_waits_for_capacity_and_shutdown_cancels_the_active_handler() {
  let path = std::env::temp_dir().join(format!("tako-raw-limit-{}.sock", std::process::id()));
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
    .try_spawn_unix_raw(&path, move |stream, _| {
      let started = started.clone();
      let release = handler_release.clone();
      Box::pin(async move {
        started.send(()).unwrap();
        release.acquire().await.unwrap().forget();
        drop(stream);
        Ok(())
      })
    })
    .await
    .unwrap();

  let _first = tokio::net::UnixStream::connect(&path).await.unwrap();
  tokio::time::timeout(Duration::from_secs(1), received.recv())
    .await
    .unwrap()
    .unwrap();

  let mut second = tokio::net::UnixStream::connect(&path).await.unwrap();
  assert!(
    tokio::time::timeout(Duration::from_millis(75), received.recv())
      .await
      .is_err(),
    "a second raw Unix handler started while max_connections was exhausted"
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
  std::fs::remove_file(path).unwrap();
}
