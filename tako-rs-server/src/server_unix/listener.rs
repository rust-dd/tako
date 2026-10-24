//! Tokio Unix socket binding with stale-socket cleanup, shared by the raw and HTTP loops.

use std::io;
use std::path::Path;

use crate::unix_path::bind_abstract;
use crate::unix_path::cleanup_stale_socket;
use crate::unix_path::is_abstract_path;

/// Binds a filesystem path, replacing a stale socket file, or an `@`-prefixed Linux abstract name.
pub(crate) async fn bind_unix_listener(path: &Path) -> io::Result<tokio::net::UnixListener> {
  if is_abstract_path(path) {
    return tokio::net::UnixListener::from_std(bind_abstract(path)?);
  }
  cleanup_stale_socket(path)?;
  tokio::net::UnixListener::bind(path)
}
