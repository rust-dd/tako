#![cfg(feature = "compio")]

use std::sync::Arc;
use std::time::Duration;

use compio::io::AsyncRead;
use tako_rs_server::CompioServer;
use tako_rs_server::ServerConfig;

#[test]
fn compio_raw_tcp_waits_for_capacity_and_shutdown_cancels_the_active_handler() {
  compio::runtime::Runtime::new().unwrap().block_on(async {
    let (started, mut received) = tokio::sync::mpsc::unbounded_channel();
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let handler_release = release.clone();
    let handle = limited_server()
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
    let _first = std::net::TcpStream::connect(address).unwrap();
    receive(&mut received).await;
    let mut second = compio::net::TcpStream::connect(address).await.unwrap();
    assert!(
      compio::time::timeout(Duration::from_millis(75), received.recv())
        .await
        .is_err()
    );
    release.add_permits(1);
    receive(&mut received).await;
    handle.trigger();
    compio::time::timeout(Duration::from_secs(1), handle.result())
      .await
      .unwrap()
      .unwrap();
    let compio::BufResult(result, _) =
      compio::time::timeout(Duration::from_secs(1), second.read(vec![0]))
        .await
        .unwrap();
    assert_eq!(result.unwrap(), 0);
  });
}

#[cfg(unix)]
#[test]
fn compio_raw_unix_waits_for_capacity_and_shutdown_cancels_the_active_handler() {
  compio::runtime::Runtime::new().unwrap().block_on(async {
    let path = std::env::temp_dir().join(format!("tako-compio-limit-{}.sock", std::process::id()));
    let (started, mut received) = tokio::sync::mpsc::unbounded_channel();
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let handler_release = release.clone();
    let handle = limited_server()
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
    let _first = std::os::unix::net::UnixStream::connect(&path).unwrap();
    receive(&mut received).await;
    let mut second = compio::net::UnixStream::connect(&path).await.unwrap();
    assert!(
      compio::time::timeout(Duration::from_millis(75), received.recv())
        .await
        .is_err()
    );
    release.add_permits(1);
    receive(&mut received).await;
    handle.trigger();
    compio::time::timeout(Duration::from_secs(1), handle.result())
      .await
      .unwrap()
      .unwrap();
    let compio::BufResult(result, _) =
      compio::time::timeout(Duration::from_secs(1), second.read(vec![0]))
        .await
        .unwrap();
    assert_eq!(result.unwrap(), 0);
    std::fs::remove_file(path).unwrap();
  });
}

#[cfg(feature = "udp")]
#[test]
fn compio_raw_udp_drops_excess_datagrams_and_resumes_after_capacity_returns() {
  compio::runtime::Runtime::new().unwrap().block_on(async {
    let (started, mut received) = tokio::sync::mpsc::unbounded_channel();
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let handler_release = release.clone();
    let handle = limited_server()
      .try_spawn_udp_raw("127.0.0.1:0", move |data, _, _| {
        let started = started.clone();
        let release = handler_release.clone();
        Box::pin(async move {
          started.send(data).unwrap();
          release.acquire().await.unwrap().forget();
        })
      })
      .unwrap();
    let address = handle.local_addr().unwrap();
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.send_to(b"first", address).unwrap();
    assert_eq!(receive(&mut received).await, b"first");
    for _ in 0..32 {
      socket.send_to(b"excess", address).unwrap();
    }
    assert!(
      compio::time::timeout(Duration::from_millis(75), received.recv())
        .await
        .is_err()
    );
    release.add_permits(1);
    compio::time::sleep(Duration::from_millis(20)).await;
    socket.send_to(b"after", address).unwrap();
    assert_eq!(receive(&mut received).await, b"after");
    handle.trigger();
    compio::time::timeout(Duration::from_secs(1), handle.result())
      .await
      .unwrap()
      .unwrap();
  });
}

fn limited_server() -> CompioServer {
  CompioServer::builder()
    .config(ServerConfig {
      max_connections: Some(1),
      drain_timeout: Duration::from_millis(20),
      ..ServerConfig::default()
    })
    .build()
}

async fn receive<T>(receiver: &mut tokio::sync::mpsc::UnboundedReceiver<T>) -> T {
  compio::time::timeout(Duration::from_secs(1), receiver.recv())
    .await
    .unwrap()
    .unwrap()
}
