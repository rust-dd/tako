#![cfg(unix)]

use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::time::Duration;

use tako_rs_core::router::Router;

struct BackloggedSocket {
  path: PathBuf,
  listener: UnixListener,
  clients: Vec<tokio::net::UnixStream>,
  inode: u64,
}

impl BackloggedSocket {
  fn new() -> Self {
    let path = std::env::temp_dir().join(format!("tako-full-backlog-{}.sock", std::process::id()));
    let listener = UnixListener::bind(&path).unwrap();
    let inode = std::fs::symlink_metadata(&path).unwrap().ino();
    let clients = tokio::runtime::Runtime::new().unwrap().block_on(async {
      let mut clients = Vec::new();
      for _ in 0..512 {
        match tokio::time::timeout(
          Duration::from_millis(2),
          tokio::net::UnixStream::connect(&path),
        )
        .await
        {
          Ok(Ok(client)) => clients.push(client),
          _ => return clients,
        }
      }
      panic!("could not fill listener backlog");
    });
    Self {
      path,
      listener,
      clients,
      inode,
    }
  }

  fn assert_preserved(&mut self) {
    assert_eq!(
      std::fs::symlink_metadata(&self.path).unwrap().ino(),
      self.inode
    );
    self.clients.clear();
    self.listener.set_nonblocking(true).unwrap();
    while self.listener.accept().is_ok() {}
    let _client = std::os::unix::net::UnixStream::connect(&self.path).unwrap();
    self
      .listener
      .accept()
      .expect("the original listener must remain reachable");
  }
}

impl Drop for BackloggedSocket {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.path);
  }
}

#[cfg(not(feature = "compio"))]
#[test]
fn overloaded_listener_is_preserved() {
  let mut socket = BackloggedSocket::new();
  tokio::runtime::Runtime::new().unwrap().block_on(async {
    let replacement = tako_rs_server::Server::builder()
      .build()
      .try_spawn_unix_http(&socket.path, Router::new())
      .await;
    let replaced = replacement.is_ok();
    if let Ok(server) = replacement {
      server.shutdown(Duration::from_secs(1)).await;
    }
    assert!(
      !replaced,
      "a full listener backlog must not permit replacement"
    );
  });
  socket.assert_preserved();
}

#[cfg(feature = "compio")]
#[test]
fn overloaded_listener_is_preserved() {
  let mut socket = BackloggedSocket::new();
  compio::runtime::Runtime::new().unwrap().block_on(async {
    let replacement = tako_rs_server::CompioServer::builder()
      .build()
      .try_spawn_unix_http(&socket.path, Router::new())
      .await;
    let replaced = replacement.is_ok();
    if let Ok(server) = replacement {
      server.shutdown(Duration::from_secs(1)).await;
    }
    assert!(
      !replaced,
      "a full listener backlog must not permit replacement"
    );
  });
  socket.assert_preserved();
}
