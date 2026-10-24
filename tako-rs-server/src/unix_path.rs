//! Runtime-agnostic Unix socket binding checks and stale-socket cleanup.

use std::io;
use std::path::Path;

/// An `@` prefix marks a Linux abstract-namespace socket.
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

/// Reports whether `path` holds a socket file; any other file, symlinks included, is an error so
/// the caller never removes it.
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

/// Unlinks a stale socket file. Losing the race to another remover counts as success, since
/// `bind(2)` still rejects the second binder.
pub(crate) fn remove_stale(path: &Path) -> io::Result<()> {
  match std::fs::remove_file(path) {
    Ok(()) => Ok(()),
    Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
    Err(e) => Err(e),
  }
}

/// Removes a socket only when no kernel endpoint is attached to its path.
pub(crate) fn cleanup_stale_socket(path: &Path) -> io::Result<()> {
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
