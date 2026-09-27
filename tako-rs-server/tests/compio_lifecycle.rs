#![cfg(feature = "compio")]

use std::io::Read;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use tako_rs_core::router::Router;
use tako_rs_server::CompioServer;
use tako_rs_server::ServerConfig;

#[test]
fn compio_idle_shutdown_and_header_timer_work() {
  compio::runtime::Runtime::new().unwrap().block_on(async {
    for partial in [false, true] {
      let state = Arc::new(());
      let mut router = Router::new();
      router.with_state(state.clone());
      router.get("/", || async { "ok" });
      let listener = compio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
      let address = listener.local_addr().unwrap();
      let handle = CompioServer::builder()
        .config(ServerConfig {
          header_read_timeout: Some(Duration::from_millis(30)),
          ..ServerConfig::default()
        })
        .build()
        .try_spawn_http(listener, router)
        .unwrap();
      assert_eq!(handle.local_addr(), Some(address));
      let (ready, received) = tokio::sync::oneshot::channel();
      let client = std::thread::spawn(move || {
        let mut stream = std::net::TcpStream::connect(address).unwrap();
        stream
          .set_read_timeout(Some(Duration::from_secs(2)))
          .unwrap();
        if partial {
          stream.write_all(b"GET / HTTP/1.1\r\nHost:").unwrap();
          let mut response = Vec::new();
          stream.read_to_end(&mut response).unwrap();
          ready.send(()).unwrap();
        } else {
          stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
          let mut response = [0; 1024];
          let read = stream.read(&mut response).unwrap();
          assert!(String::from_utf8_lossy(&response[..read]).starts_with("HTTP/1.1 200"));
          ready.send(()).unwrap();
          let mut remaining = Vec::new();
          stream.read_to_end(&mut remaining).unwrap();
        }
      });
      compio::time::timeout(Duration::from_secs(2), received)
        .await
        .unwrap()
        .unwrap();
      compio::time::timeout(
        Duration::from_secs(1),
        handle.shutdown(Duration::from_secs(5)),
      )
      .await
      .unwrap();
      client.join().unwrap();
      assert_eq!(Arc::strong_count(&state), 1);
    }
  });
}
