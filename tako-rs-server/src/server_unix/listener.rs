//! Unix socket binding, abstract-namespace detection, stale-socket cleanup,
//! and the peer-address type shared by the raw and HTTP serve loops.

use std::io;
use std::path::Path;

/// Returns true if `path`'s string form starts with `@`, marking it as a
/// Linux abstract-namespace socket.
#[inline]
pub(crate) fn is_abstract_path(path: &Path) -> bool {
  path.to_str().is_some_and(|s| s.starts_with('@'))
}

/// Bind a `tokio::net::UnixListener` for either a filesystem path or a Linux
/// abstract path (`@`-prefixed). Filesystem paths get the stale-socket
/// cleanup; abstract paths don't.
pub(crate) async fn bind_unix_listener(path: &Path) -> io::Result<tokio::net::UnixListener> {
  if is_abstract_path(path) {
    #[cfg(target_os = "linux")]
    {
      use std::os::linux::net::SocketAddrExt;
      let name = &path.to_str().unwrap().as_bytes()[1..];
      let addr = std::os::unix::net::SocketAddr::from_abstract_name(name)?;
      let std_listener = std::os::unix::net::UnixListener::bind_addr(&addr)?;
      std_listener.set_nonblocking(true)?;
      return tokio::net::UnixListener::from_std(std_listener);
    }
    #[cfg(not(target_os = "linux"))]
    {
      return Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "abstract Unix socket paths (`@`-prefixed) are Linux-only",
      ));
    }
  }
  cleanup_stale_socket(path).await?;
  tokio::net::UnixListener::bind(path)
}

/// Peer address information for Unix domain socket connections.
///
/// Inserted into request extensions for HTTP-over-UDS connections.
/// Handlers can access it via `req.extensions().get::<UnixPeerAddr>()`.
#[derive(Debug, Clone)]
pub struct UnixPeerAddr {
  /// The filesystem path of the peer socket, if available.
  /// Most client connections are unnamed (None).
  pub path: Option<std::path::PathBuf>,
}

/// Removes a stale socket file if it exists and is not actively in use.
///
/// Probes the socket via the async `tokio::net::UnixStream::connect` so the
/// runtime worker isn't blocked by the previous synchronous `connect()`
/// while another peer's accept queue is draining. A 50ms connect deadline
/// stops a malicious or stuck peer from holding the bind forever.
///
/// **Symlink safety**: `symlink_metadata` + `FileTypeExt::is_socket` make
/// sure the path is itself an `AF_UNIX` socket file before we touch it. If
/// the path turns out to be a regular file, a directory, or a symlink to
/// something else (e.g. `/etc/passwd`), we surface an error instead of
/// blindly `remove_file`-ing it — replacing the socket path with a symlink
/// is otherwise a textbook escalation trap.
///
/// **Concurrency**: a sibling process may remove the same stale file in
/// parallel. The kernel's `bind(2)` is the authoritative race-resolver — it
/// rejects the second binder with `EADDRINUSE`. This helper therefore treats
/// a `remove_file` that races against a sibling (`NotFound`) as success and
/// lets the caller proceed to `bind`. For stronger guarantees use an
/// out-of-band lock (supervisor sequencing, advisory file lock).
async fn cleanup_stale_socket(path: &Path) -> std::io::Result<()> {
  use std::os::unix::fs::FileTypeExt;
  use std::time::Duration;

  let meta = match std::fs::symlink_metadata(path) {
    Ok(m) => m,
    Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
    Err(e) => return Err(e),
  };
  if !meta.file_type().is_socket() {
    return Err(std::io::Error::new(
      std::io::ErrorKind::AlreadyExists,
      format!(
        "{} exists but is not a unix socket; refusing to remove",
        path.display()
      ),
    ));
  }
  let connect = tokio::net::UnixStream::connect(path);
  match tokio::time::timeout(Duration::from_millis(50), connect).await {
    Ok(Ok(_)) => Err(std::io::Error::new(
      std::io::ErrorKind::AddrInUse,
      format!("Unix socket {} is already in use", path.display()),
    )),
    // Connect failed within the deadline (stale socket) or the deadline
    // fired (peer hung mid-handshake) — both mean the path is no longer
    // serving a live process; safe to unlink.
    Ok(Err(_)) | Err(_) => match std::fs::remove_file(path) {
      Ok(()) => Ok(()),
      // Another process removed the same stale file between our metadata
      // probe and the unlink syscall — bind() will resolve the rest.
      Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
      Err(e) => Err(e),
    },
  }
}
