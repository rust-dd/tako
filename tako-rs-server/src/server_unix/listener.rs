//! Tokio Unix socket binding with stale-socket cleanup, shared by the raw and HTTP loops.

use std::io;
use std::path::Path;

use crate::unix_path::PROBE_TIMEOUT;
use crate::unix_path::bind_abstract;
use crate::unix_path::existing_socket;
use crate::unix_path::in_use;
use crate::unix_path::is_abstract_path;
use crate::unix_path::remove_stale;

/// Binds a filesystem path, replacing a stale socket file, or an `@`-prefixed Linux abstract name.
pub(crate) async fn bind_unix_listener(path: &Path) -> io::Result<tokio::net::UnixListener> {
  if is_abstract_path(path) {
    return tokio::net::UnixListener::from_std(bind_abstract(path)?);
  }
  cleanup_stale_socket(path).await?;
  tokio::net::UnixListener::bind(path)
}

/// Removes the socket file at `path` unless a live process still accepts on it.
async fn cleanup_stale_socket(path: &Path) -> io::Result<()> {
  if !existing_socket(path)? {
    return Ok(());
  }
  let connect = tokio::net::UnixStream::connect(path);
  match tokio::time::timeout(PROBE_TIMEOUT, connect).await {
    Ok(Ok(_)) => Err(in_use(path)),
    // A refused connect or an expired deadline both mean no live process
    // serves the path any more.
    Ok(Err(_)) | Err(_) => remove_stale(path),
  }
}
