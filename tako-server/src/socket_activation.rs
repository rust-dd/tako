#![cfg(feature = "socket-activation")]
#![cfg_attr(docsrs, doc(cfg(feature = "socket-activation")))]

//! systemd / s6 / catflap socket activation.
//!
//! When this process is launched by an init system (systemd, s6, runit) using
//! socket activation, the listening sockets are inherited via the `LISTEN_FDS`
//! and `LISTEN_PID` environment variables. This module wraps the [`listenfd`]
//! crate and converts the inherited file descriptors into the tokio types the
//! Tako transports expect.
//!
//! # Example (systemd)
//!
//! ```rust,no_run
//! # #[cfg(feature = "socket-activation")]
//! # async fn _ex() -> std::io::Result<()> {
//! use tako_server::socket_activation::ListenFds;
//! use tokio::net::TcpListener;
//!
//! let mut fds = ListenFds::from_env();
//! let listener = match fds.tcp_listener(0)? {
//!   Some(l) => l,
//!   None => TcpListener::bind("0.0.0.0:8080").await?,
//! };
//! // Hand `listener` to `Server::spawn_http(listener, router)` …
//! # Ok(())
//! # }
//! ```
//!
//! Multiple inherited sockets (e.g. one TCP for HTTP, one Unix for IPC) are
//! addressed by index in the order they were declared in the unit file.

use std::io;

/// Wrapper around [`listenfd::ListenFd`] that returns tokio listener types
/// directly so the inherited sockets plug into the existing `Server::spawn_*`
/// methods without conversion ceremony.
pub struct ListenFds(listenfd::ListenFd);

impl std::fmt::Debug for ListenFds {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("ListenFds").field("len", &self.0.len()).finish()
  }
}

impl ListenFds {
  /// Read `LISTEN_FDS` / `LISTEN_PID` (or s6 / catflap equivalents) from the
  /// environment.
  #[must_use]
  pub fn from_env() -> Self {
    Self(listenfd::ListenFd::from_env())
  }

  /// Number of inherited file descriptors detected.
  #[must_use]
  pub fn len(&self) -> usize {
    self.0.len()
  }

  /// True if no inherited descriptors are available.
  #[must_use]
  pub fn is_empty(&self) -> bool {
    self.0.len() == 0
  }

  /// Take the inherited TCP listener at `index` and convert it to a
  /// `tokio::net::TcpListener`. Returns `Ok(None)` when no socket is at the
  /// index or the FD is not a TCP listener; returns `Err` if the conversion
  /// fails.
  pub fn tcp_listener(&mut self, index: usize) -> io::Result<Option<tokio::net::TcpListener>> {
    let Some(std_listener) = self.0.take_tcp_listener(index)? else {
      return Ok(None);
    };
    std_listener.set_nonblocking(true)?;
    Ok(Some(tokio::net::TcpListener::from_std(std_listener)?))
  }

  /// Take the inherited Unix listener at `index` and convert it to a
  /// `tokio::net::UnixListener`. Linux-only.
  #[cfg(unix)]
  pub fn unix_listener(&mut self, index: usize) -> io::Result<Option<tokio::net::UnixListener>> {
    let Some(std_listener) = self.0.take_unix_listener(index)? else {
      return Ok(None);
    };
    std_listener.set_nonblocking(true)?;
    Ok(Some(tokio::net::UnixListener::from_std(std_listener)?))
  }

  /// Take the inherited UDP socket at `index` and convert it to a
  /// `tokio::net::UdpSocket`. Useful for HTTP/3 / QUIC endpoints supplied by
  /// systemd's `ListenDatagram=`.
  pub fn udp_socket(&mut self, index: usize) -> io::Result<Option<tokio::net::UdpSocket>> {
    let Some(std_socket) = self.0.take_udp_socket(index)? else {
      return Ok(None);
    };
    std_socket.set_nonblocking(true)?;
    Ok(Some(tokio::net::UdpSocket::from_std(std_socket)?))
  }
}
