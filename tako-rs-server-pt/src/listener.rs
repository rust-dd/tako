use std::io;
use std::net::SocketAddr;

use socket2::Domain;
use socket2::Protocol;
use socket2::Socket;
use socket2::Type;
use tokio::net::TcpListener;

/// One-shot platform-capability warning. `SO_REUSEPORT` behaves like
/// kernel-level load balancing only on Linux; macOS / *BSD ignore the load-
/// balance semantic (last-binder-wins), and Windows lacks the option entirely.
fn warn_reuseport_platform_once() {
  static WARNED: std::sync::Once = std::sync::Once::new();
  WARNED.call_once(|| {
    #[cfg(target_os = "linux")]
    {
      // No-op: SO_REUSEPORT is the supported configuration.
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
      tracing::warn!(
        "tako-server-pt: SO_REUSEPORT is being used on a non-Linux Unix \
         platform. The kernel typically sends incoming connections only to \
         the most recent binder, so multi-worker thread-per-core mode will \
         not load-balance correctly. Use a single worker or run on Linux."
      );
    }
    #[cfg(windows)]
    {
      tracing::warn!(
        "tako-server-pt: SO_REUSEPORT does not exist on Windows. Only the \
         first worker will accept connections; subsequent worker binds will \
         fail with EADDRINUSE. Use a single worker on Windows."
      );
    }
  });
}

fn bind_reuseport_std(addr: SocketAddr, backlog: i32) -> io::Result<std::net::TcpListener> {
  warn_reuseport_platform_once();
  let domain = if addr.is_ipv4() {
    Domain::IPV4
  } else {
    Domain::IPV6
  };
  let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
  socket.set_reuse_address(true)?;
  // `socket2::set_reuse_port` is gated to Unix targets only; on Linux it's a
  // genuine kernel load-balancer, on macOS / BSD it's a no-op-equivalent
  // (last-binder-wins), on Windows the underlying SO_REUSEPORT does not
  // exist so the call is omitted entirely.
  #[cfg(unix)]
  socket.set_reuse_port(true)?;
  socket.set_nonblocking(true)?;
  socket.bind(&addr.into())?;
  socket.listen(backlog)?;
  Ok(socket.into())
}

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
