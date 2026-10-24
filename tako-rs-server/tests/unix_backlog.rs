#![cfg(unix)]

use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::time::Duration;

use socket2::Domain;
use socket2::SockAddr;
use socket2::Socket;
use socket2::Type;
use tako_rs_core::router::Router;

struct BackloggedSocket {
  path: PathBuf,
  listener: UnixListener,
  clients: Vec<Socket>,
  inode: u64,
}

impl BackloggedSocket {
  fn new() -> Self {
    let path = std::env::temp_dir().join(format!("tako-full-backlog-{}.sock", std::process::id()));
    let address = SockAddr::unix(&path).unwrap();
    let listener = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap();
    listener.bind(&address).unwrap();
    listener.listen(1).unwrap();
    let inode = std::fs::symlink_metadata(&path).unwrap().ino();
    let mut socket = Self {
      path,
      listener: listener.into(),
      clients: Vec::new(),
      inode,
    };
    for _ in 0..16 {
      let client = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap();
      client.set_nonblocking(true).unwrap();
      match client.connect(&address) {
        Ok(()) => socket.clients.push(client),
        Err(error) => {
          assert!(
            matches!(
              error.kind(),
              ErrorKind::WouldBlock | ErrorKind::ConnectionRefused
            ),
            "unexpected error while filling listener backlog: {error}"
          );
          assert!(
            !socket.clients.is_empty(),
            "listener never queued a connection"
          );
          return socket;
        }
      }
    }
    panic!("could not fill listener backlog");
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
