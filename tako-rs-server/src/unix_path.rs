//! Runtime-agnostic parts of Unix socket binding: abstract-namespace
//! detection and the checks that decide whether an existing path may be
//! replaced. The Tokio and Compio listeners add their own connect probe.

use std::io;
use std::path::Path;
use std::time::Duration;

/// How long a stale-socket probe waits for a previous owner to accept.
///
/// A short deadline stops a malicious or stuck peer from holding the bind
/// forever.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_millis(50);

/// Returns true if `path`'s string form starts with `@`, marking it as a
/// Linux abstract-namespace socket.
#[inline]
pub(crate) fn is_abstract_path(path: &Path) -> bool {
  path.to_str().is_some_and(|s| s.starts_with('@'))
}

/// Binds a nonblocking std listener on the Linux abstract name behind an
/// `@`-prefixed path.
pub(crate) fn bind_abstract(path: &Path) -> io::Result<std::os::unix::net::UnixListener> {
  #[cfg(target_os = "linux")]
  {
    use std::os::linux::net::SocketAddrExt;
    let name = &path.to_str().unwrap().as_bytes()[1..];
    let addr = std::os::unix::net::SocketAddr::from_abstract_name(name)?;
    let listener = std::os::unix::net::UnixListener::bind_addr(&addr)?;
    listener.set_nonblocking(true)?;
    Ok(listener)
  }
  #[cfg(not(target_os = "linux"))]
  {
    let _ = path;
    Err(io::Error::new(
      io::ErrorKind::Unsupported,
      "abstract Unix socket paths (`@`-prefixed) are Linux-only",
    ))
  }
}

/// Reports whether `path` holds a socket file that may be stale.
///
/// **Symlink safety**: `symlink_metadata` + `FileTypeExt::is_socket` make
/// sure the path is itself an `AF_UNIX` socket file before anyone touches it.
/// If the path is a regular file, a directory, or a symlink to something else
/// (e.g. `/etc/passwd`), this returns an error instead of letting the caller
/// `remove_file` it — replacing the socket path with a symlink is otherwise a
/// textbook escalation trap.
pub(crate) fn existing_socket(path: &Path) -> io::Result<bool> {
  use std::os::unix::fs::FileTypeExt;

  let meta = match std::fs::symlink_metadata(path) {
    Ok(m) => m,
    Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
    Err(e) => return Err(e),
  };
  if !meta.file_type().is_socket() {
    return Err(io::Error::new(
      io::ErrorKind::AlreadyExists,
      format!(
        "{} exists but is not a unix socket; refusing to remove",
        path.display()
      ),
    ));
  }
  Ok(true)
}

/// The error for a socket path that another process still serves.
pub(crate) fn in_use(path: &Path) -> io::Error {
  io::Error::new(
    io::ErrorKind::AddrInUse,
    format!("Unix socket {} is already in use", path.display()),
  )
}

/// Unlinks a stale socket file.
///
/// **Concurrency**: a sibling process may remove the same stale file in
/// parallel. The kernel's `bind(2)` is the authoritative race-resolver — it
/// rejects the second binder with `EADDRINUSE` — so a `remove_file` that loses
/// that race (`NotFound`) counts as success. For stronger guarantees use an
/// out-of-band lock (supervisor sequencing, advisory file lock).
pub(crate) fn remove_stale(path: &Path) -> io::Result<()> {
  match std::fs::remove_file(path) {
    Ok(()) => Ok(()),
    Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
    Err(e) => Err(e),
  }
}
