//! Tokio Unix socket binding with stale-socket cleanup, shared by the raw and
//! HTTP serve loops.

use std::io;
use std::path::Path;

use crate::unix_path::PROBE_TIMEOUT;
use crate::unix_path::bind_abstract;
use crate::unix_path::existing_socket;
use crate::unix_path::in_use;
pub(crate) use crate::unix_path::is_abstract_path;
use crate::unix_path::remove_stale;

/// Bind a `tokio::net::UnixListener` for either a filesystem path or a Linux
/// abstract path (`@`-prefixed). Filesystem paths get the stale-socket
/// cleanup; abstract paths don't.
pub(crate) async fn bind_unix_listener(path: &Path) -> io::Result<tokio::net::UnixListener> {
  if is_abstract_path(path) {
    return tokio::net::UnixListener::from_std(bind_abstract(path)?);
  }
  cleanup_stale_socket(path).await?;
  tokio::net::UnixListener::bind(path)
}

/// Removes a stale socket file if it exists and is not actively in use.
///
/// Probes the socket via the async `tokio::net::UnixStream::connect` so the
/// runtime worker isn't blocked by a synchronous `connect()` while another
/// peer's accept queue is draining.
async fn cleanup_stale_socket(path: &Path) -> io::Result<()> {
  if !existing_socket(path)? {
    return Ok(());
  }
  let connect = tokio::net::UnixStream::connect(path);
  match tokio::time::timeout(PROBE_TIMEOUT, connect).await {
    Ok(Ok(_)) => Err(in_use(path)),
    // Connect failed within the deadline (stale socket) or the deadline
    // fired (peer hung mid-handshake) — both mean the path is no longer
    // serving a live process; safe to unlink.
    Ok(Err(_)) | Err(_) => remove_stale(path),
  }
}
