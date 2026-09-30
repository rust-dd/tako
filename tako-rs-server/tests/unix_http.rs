#![cfg(unix)]

use std::io::Read;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::conn_info::UnixPeerAddr;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;

fn socket_path(name: &str) -> PathBuf {
  static NEXT: AtomicUsize = AtomicUsize::new(0);
  let id = NEXT.fetch_add(1, Ordering::Relaxed);
  std::env::temp_dir().join(format!("tako-{name}-{}-{id}.sock", std::process::id()))
}

fn router() -> Router {
  let mut router = Router::new();
  router.get("/peer", |req: Request| async move {
    let info = req.extensions().get::<ConnInfo>().unwrap();
    let peer = req.extensions().get::<UnixPeerAddr>().is_some();
    format!("{:?} peer={peer}", info.transport)
  });
  router
}

fn get(path: &Path, uri: &str) -> String {
  let mut stream = UnixStream::connect(path).unwrap();
  stream
    .set_read_timeout(Some(Duration::from_secs(2)))
    .unwrap();
  write!(
    stream,
    "GET {uri} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
  )
  .unwrap();
  let mut response = String::new();
  stream.read_to_string(&mut response).unwrap();
  response
}

fn leave_stale_socket(path: &Path) {
  drop(std::os::unix::net::UnixListener::bind(path).unwrap());
  assert!(path.exists());
}

fn echo(path: &Path) -> Vec<u8> {
  for _ in 0..200 {
    if let Ok(mut stream) = UnixStream::connect(path) {
      stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
      stream.write_all(b"ping").unwrap();
      let mut buf = [0u8; 4];
      stream.read_exact(&mut buf).unwrap();
      return buf.to_vec();
    }
    std::thread::sleep(Duration::from_millis(5));
  }
  panic!("raw unix server never accepted a connection");
}

#[cfg(not(feature = "compio"))]
mod tokio_runtime {
  use tako_rs_server::Server;

  use super::*;

  #[tokio::test]
  async fn serves_http_and_cleans_up_the_socket() {
    let path = socket_path("tokio");
    leave_stale_socket(&path);
    let handle = Server::builder()
      .build()
      .try_spawn_unix_http(&path, router())
      .await
      .unwrap();
    let client_path = path.clone();
    let response = tokio::task::spawn_blocking(move || get(&client_path, "/peer"))
      .await
      .unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.ends_with("Unix peer=true"), "{response}");
    handle.shutdown(Duration::from_secs(1)).await;
    assert!(!path.exists());
  }

  #[tokio::test]
  async fn refuses_to_replace_a_regular_file() {
    let path = socket_path("tokio-file");
    std::fs::write(&path, b"not a socket").unwrap();
    let error = Server::builder()
      .build()
      .try_spawn_unix_http(&path, router())
      .await
      .expect_err("binding over a regular file must fail");
    assert!(error.to_string().contains("not a unix socket"), "{error}");
    std::fs::remove_file(&path).unwrap();
  }

  #[tokio::test]
  async fn raw_handler_echoes_and_drains() {
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;

    let path = socket_path("tokio-raw");
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server_path = path.clone();
    let server = tokio::spawn(async move {
      tako_rs_server::server_unix::serve_unix_with_shutdown(
        &server_path,
        |mut stream, _addr| {
          Box::pin(async move {
            let mut buf = [0u8; 4];
            stream.read_exact(&mut buf).await?;
            stream.write_all(&buf).await
          })
        },
        async move {
          let _ = stopped.await;
        },
      )
      .await
    });
    let client_path = path.clone();
    let echoed = tokio::task::spawn_blocking(move || echo(&client_path))
      .await
      .unwrap();
    assert_eq!(&echoed, b"ping");
    stop.send(()).unwrap();
    server.await.unwrap().unwrap();
  }
}

#[cfg(feature = "compio")]
mod compio_runtime {
  use tako_rs_server::CompioServer;

  use super::*;

  #[test]
  fn serves_http_and_cleans_up_the_socket() {
    compio::runtime::Runtime::new().unwrap().block_on(async {
      let path = socket_path("compio");
      leave_stale_socket(&path);
      let handle = CompioServer::builder()
        .build()
        .try_spawn_unix_http(&path, router())
        .await
        .unwrap();
      let (sender, received) = tokio::sync::oneshot::channel();
      let client_path = path.clone();
      let client = std::thread::spawn(move || {
        sender.send(get(&client_path, "/peer")).unwrap();
      });
      let response = compio::time::timeout(Duration::from_secs(5), received)
        .await
        .expect("unix client timed out")
        .unwrap();
      client.join().unwrap();
      assert!(response.starts_with("HTTP/1.1 200"), "{response}");
      assert!(response.ends_with("Unix peer=true"), "{response}");
      compio::time::timeout(
        Duration::from_secs(2),
        handle.shutdown(Duration::from_secs(1)),
      )
      .await
      .expect("unix server did not shut down");
      assert!(!path.exists());
    });
  }

  #[test]
  fn refuses_to_replace_a_regular_file() {
    compio::runtime::Runtime::new().unwrap().block_on(async {
      let path = socket_path("compio-file");
      std::fs::write(&path, b"not a socket").unwrap();
      let error = CompioServer::builder()
        .build()
        .try_spawn_unix_http(&path, router())
        .await
        .expect_err("binding over a regular file must fail");
      assert!(error.to_string().contains("not a unix socket"), "{error}");
      std::fs::remove_file(&path).unwrap();
    });
  }

  #[test]
  fn refuses_a_socket_that_is_in_use() {
    compio::runtime::Runtime::new().unwrap().block_on(async {
      let path = socket_path("compio-busy");
      let _live = std::os::unix::net::UnixListener::bind(&path).unwrap();
      let error = CompioServer::builder()
        .build()
        .try_spawn_unix_http(&path, router())
        .await
        .expect_err("binding over a live socket must fail");
      assert!(error.to_string().contains("already in use"), "{error}");
      std::fs::remove_file(&path).unwrap();
    });
  }

  #[test]
  fn raw_handler_echoes_and_drains() {
    use compio::io::AsyncReadExt;
    use compio::io::AsyncWriteExt;

    compio::runtime::Runtime::new().unwrap().block_on(async {
      let path = socket_path("compio-raw");
      let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
      let server_path = path.clone();
      let server = compio::runtime::spawn(async move {
        tako_rs_server::server_unix::serve_unix_with_shutdown(
          &server_path,
          |mut stream, peer| {
            Box::pin(async move {
              assert!(peer.path.is_none());
              let compio::BufResult(result, buf) = stream.read_exact(vec![0u8; 4]).await;
              result?;
              let compio::BufResult(result, _) = stream.write_all(buf).await;
              result
            })
          },
          async move {
            let _ = stopped.await;
          },
        )
        .await
      });
      let (sender, received) = tokio::sync::oneshot::channel();
      let client_path = path.clone();
      let client = std::thread::spawn(move || sender.send(echo(&client_path)).unwrap());
      let echoed = compio::time::timeout(Duration::from_secs(5), received)
        .await
        .expect("raw unix client timed out")
        .unwrap();
      client.join().unwrap();
      assert_eq!(&echoed, b"ping");
      stop.send(()).unwrap();
      compio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("raw unix server did not stop")
        .unwrap()
        .unwrap();
    });
  }
}
