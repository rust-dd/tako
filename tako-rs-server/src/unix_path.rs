//! Runtime-agnostic Unix socket binding checks and stale-socket cleanup.

use std::fs::OpenOptions;
use std::fs::TryLockError;
use std::io;
use std::os::unix::net::UnixListener;
use std::path::Path;

/// Binds under a cross-process lock so stale cleanup cannot unlink a new listener.
pub(crate) fn bind_listener(path: &Path) -> io::Result<UnixListener> {
  if is_abstract_path(path) {
    return bind_abstract(path);
  }

  let mut lock_path = path.as_os_str().to_os_string();
  lock_path.push(".lock");
  // Keep the lock inode permanently: unlinking it lets another opener bypass a held lock.
  let lock = OpenOptions::new()
    .read(true)
    .write(true)
    .create(true)
    .truncate(false)
    .open(lock_path)?;
  match lock.try_lock() {
    Ok(()) => {}
    Err(TryLockError::WouldBlock) => return Err(in_use(path)),
    Err(TryLockError::Error(error)) => return Err(error),
  }
  cleanup_stale_socket(path)?;
  UnixListener::bind(path)
}

/// An `@` prefix marks a Linux abstract-namespace socket.
#[inline]
pub(crate) fn is_abstract_path(path: &Path) -> bool {
  path.to_str().is_some_and(|s| s.starts_with('@'))
}

/// Binds the Linux abstract name behind an `@`-prefixed path.
fn bind_abstract(path: &Path) -> io::Result<UnixListener> {
  #[cfg(target_os = "linux")]
  {
    use std::os::linux::net::SocketAddrExt;
    let name = &path.to_str().unwrap().as_bytes()[1..];
    let addr = std::os::unix::net::SocketAddr::from_abstract_name(name)?;
    UnixListener::bind_addr(&addr)
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

/// Reports whether `path` holds a socket file; any other file, symlinks included, is an error so
/// the caller never removes it.
fn existing_socket(path: &Path) -> io::Result<bool> {
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
fn in_use(path: &Path) -> io::Error {
  io::Error::new(
    io::ErrorKind::AddrInUse,
    format!("Unix socket {} is already in use", path.display()),
  )
}

/// Unlinks a stale socket while the startup lock excludes other cooperating binders.
fn remove_stale(path: &Path) -> io::Result<()> {
  match std::fs::remove_file(path) {
    Ok(()) => Ok(()),
    Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
    Err(e) => Err(e),
  }
}

/// Removes a socket only when no kernel endpoint is attached to its path.
fn cleanup_stale_socket(path: &Path) -> io::Result<()> {
  if !existing_socket(path)? {
    return Ok(());
  }
  // A stream connect can return ConnectionRefused for a full live backlog on BSD.
  // A datagram probe checks the endpoint type before consulting that backlog.
  let probe = std::os::unix::net::UnixDatagram::unbound()?;
  probe.set_nonblocking(true)?;
  match probe.connect(path) {
    Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => remove_stale(path),
    _ => Err(in_use(path)),
  }
}
