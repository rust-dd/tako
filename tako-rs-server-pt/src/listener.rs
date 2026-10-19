use std::io;
use std::net::SocketAddr;

use socket2::Domain;
use socket2::Protocol;
use socket2::Socket;
use socket2::Type;
#[cfg(not(feature = "compio"))]
use tokio::net::TcpListener;

/// Warns once off Linux, the only platform where `SO_REUSEPORT` balances load: macOS and the BSDs
/// send every connection to the last binder, and Windows lacks the option.
pub(crate) fn warn_reuseport_platform_once() {
  static WARNED: std::sync::Once = std::sync::Once::new();
  WARNED.call_once(|| {
    #[cfg(all(unix, not(target_os = "linux")))]
    {
      tracing::warn!(
        "tako-server-pt: SO_REUSEPORT is being used on a non-Linux Unix \
         platform with `balance_connections` disabled. The kernel typically \
         sends incoming connections only to the most recent binder, so \
         multi-worker thread-per-core mode will not load-balance. Enable \
         `balance_connections`, use a single worker, or run on Linux."
      );
    }
    #[cfg(windows)]
    {
      tracing::warn!(
        "tako-server-pt: SO_REUSEPORT does not exist on Windows. With \
         `balance_connections` disabled, only the first worker accepts \
         connections; subsequent worker binds fail with EADDRINUSE. Enable \
         `balance_connections` or use a single worker on Windows."
      );
    }
  });
}

fn bind_reuseport_std(addr: SocketAddr, backlog: i32) -> io::Result<std::net::TcpListener> {
  let domain = if addr.is_ipv4() {
    Domain::IPV4
  } else {
    Domain::IPV6
  };
  let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
  socket.set_reuse_address(true)?;
  // Windows has no `SO_REUSEPORT`.
  #[cfg(unix)]
  socket.set_reuse_port(true)?;
  socket.set_nonblocking(true)?;
  socket.bind(&addr.into())?;
  socket.listen(backlog)?;
  Ok(socket.into())
}

#[cfg(not(feature = "compio"))]
pub(crate) fn bind_reuseport(addr: SocketAddr, backlog: i32) -> io::Result<TcpListener> {
  TcpListener::from_std(bind_reuseport_std(addr, backlog)?)
}

#[cfg(feature = "compio")]
pub(crate) fn bind_reuseport_compio(
  addr: SocketAddr,
  backlog: i32,
) -> io::Result<compio::net::TcpListener> {
  compio::net::TcpListener::from_std(bind_reuseport_std(addr, backlog)?)
}
