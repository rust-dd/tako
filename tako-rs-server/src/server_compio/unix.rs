//! Unix domain socket listener for the compio HTTP server.

use std::io;
use std::path::Path;
use std::path::PathBuf;

use tako_rs_core::conn_info::UnixPeerAddr;

use super::accept::Listener;
use super::connection::Peer;
use crate::unix_path::bind_abstract;
use crate::unix_path::cleanup_stale_socket;
use crate::unix_path::is_abstract_path;

/// A bound Unix socket plus the path it removes after shutdown.
pub(crate) struct UnixSocketListener {
  inner: compio::net::UnixListener,
  path: PathBuf,
}

impl UnixSocketListener {
  pub(crate) async fn bind(path: PathBuf) -> io::Result<Self> {
    let inner = bind_listener(&path).await?;
    Ok(Self { inner, path })
  }
}

/// Binds `path`, replacing a stale socket file but never a live socket or a
/// non-socket file. `@`-prefixed paths bind the Linux abstract namespace.
pub(crate) async fn bind_listener(path: &Path) -> io::Result<compio::net::UnixListener> {
  if is_abstract_path(path) {
    return compio::net::UnixListener::from_std(bind_abstract(path)?);
  }
  cleanup_stale_socket(path)?;
  compio::net::UnixListener::bind(path).await
}

/// Peer address of an accepted connection, from its bound path if any.
pub(crate) fn peer_addr(path: Option<&Path>) -> UnixPeerAddr {
  UnixPeerAddr {
    path: path.map(Path::to_path_buf),
  }
}

impl Listener for UnixSocketListener {
  type Stream = compio::net::UnixStream;

  fn describe(&self) -> io::Result<String> {
    Ok(self.path.display().to_string())
  }

  fn kind(&self) -> &'static str {
    "unix"
  }

  async fn next_connection(&self) -> io::Result<(Self::Stream, Peer)> {
    let (stream, addr) = self.inner.accept().await?;
    Ok((stream, Peer::Unix(peer_addr(addr.as_pathname()))))
  }

  fn finish(&self) {
    if !is_abstract_path(&self.path) {
      let _ = std::fs::remove_file(&self.path);
    }
  }
}
