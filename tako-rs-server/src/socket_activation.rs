//! Listening sockets inherited through systemd, s6 or catflap socket activation (`LISTEN_FDS`),
//! as Tokio types; indexes follow the order the unit file declares them in.
//!
//! # Example (systemd)
//!
//! ```rust,no_run
//! # #[cfg(feature = "socket-activation")]
//! # async fn _ex() -> std::io::Result<()> {
//! use tako_rs_server::socket_activation::ListenFds;
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

use std::io;

/// Inherited sockets, returned as the Tokio types the `Server::spawn_*` methods take.
pub struct ListenFds(listenfd::ListenFd);

impl std::fmt::Debug for ListenFds {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("ListenFds")
      .field("len", &self.0.len())
      .finish()
  }
}

impl ListenFds {
  /// Reads `LISTEN_FDS` and `LISTEN_PID`, or the s6 and catflap equivalents.
  #[must_use]
  pub fn from_env() -> Self {
    Self(listenfd::ListenFd::from_env())
  }

  /// Number of inherited file descriptors.
  #[must_use]
  pub fn len(&self) -> usize {
    self.0.len()
  }

  /// True when no descriptor was inherited.
  #[must_use]
  pub fn is_empty(&self) -> bool {
    self.0.len() == 0
  }

  /// Takes the TCP listener at `index`: `Ok(None)` when nothing is left there, an error when it
  /// holds another kind of socket.
  pub fn tcp_listener(&mut self, index: usize) -> io::Result<Option<tokio::net::TcpListener>> {
    let Some(std_listener) = self.0.take_tcp_listener(index)? else {
      return Ok(None);
    };
    std_listener.set_nonblocking(true)?;
    Ok(Some(tokio::net::TcpListener::from_std(std_listener)?))
  }

  /// Takes the Unix listener at `index`: `Ok(None)` when nothing is left there, an error when it
  /// holds another kind of socket.
  #[cfg(unix)]
  pub fn unix_listener(&mut self, index: usize) -> io::Result<Option<tokio::net::UnixListener>> {
    let Some(std_listener) = self.0.take_unix_listener(index)? else {
      return Ok(None);
    };
    std_listener.set_nonblocking(true)?;
    Ok(Some(tokio::net::UnixListener::from_std(std_listener)?))
  }

  /// Takes the UDP socket at `index`, e.g. from systemd's `ListenDatagram=`: `Ok(None)` when
  /// nothing is left there, an error when it holds another kind of socket.
  pub fn udp_socket(&mut self, index: usize) -> io::Result<Option<tokio::net::UdpSocket>> {
    let Some(std_socket) = self.0.take_udp_socket(index)? else {
      return Ok(None);
    };
    std_socket.set_nonblocking(true)?;
    Ok(Some(tokio::net::UdpSocket::from_std(std_socket)?))
  }
}
