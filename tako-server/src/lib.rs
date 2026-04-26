#![cfg_attr(docsrs, feature(doc_cfg))]

//! Server bootstrap for the Tako framework.
//!
//! Hosts every concrete listener implementation (HTTP/1, TLS, HTTP/3, raw TCP,
//! UDP, Unix sockets, plus the compio variants) and the PROXY protocol parser.
//! Re-exported under the original `tako::*` paths via the umbrella crate.

use std::io::ErrorKind;
use std::io::Write;
use std::io::{self};
use std::net::SocketAddr;
use std::str::FromStr;

#[cfg(not(feature = "compio"))]
mod server;

#[cfg(not(feature = "compio"))]
pub use server::serve;
#[cfg(not(feature = "compio"))]
pub use server::serve_with_shutdown;

#[cfg(feature = "compio")]
#[cfg_attr(docsrs, doc(cfg(feature = "compio")))]
pub mod server_compio;
#[cfg(feature = "compio")]
pub use server_compio::serve;
#[cfg(feature = "compio")]
pub use server_compio::serve_with_shutdown;

/// TLS/SSL server implementation for secure connections.
#[cfg(all(not(feature = "compio-tls"), feature = "tls"))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub mod server_tls;
#[cfg(all(not(feature = "compio"), feature = "tls"))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use server_tls::serve_tls;
#[cfg(all(not(feature = "compio"), feature = "tls"))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use server_tls::serve_tls_with_shutdown;

#[cfg(feature = "compio-tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "compio-tls")))]
pub mod server_tls_compio;
#[cfg(feature = "compio-tls")]
pub use server_tls_compio::serve_tls;
#[cfg(feature = "compio-tls")]
pub use server_tls_compio::serve_tls_with_shutdown;

/// HTTP/3 server implementation using QUIC transport.
#[cfg(all(feature = "http3", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub mod server_h3;
#[cfg(all(feature = "http3", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub use server_h3::serve_h3;
#[cfg(all(feature = "http3", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub use server_h3::serve_h3_with_shutdown;

/// Raw TCP server for handling arbitrary TCP connections.
pub mod server_tcp;

/// UDP datagram server for handling raw UDP packets.
pub mod server_udp;

/// Unix Domain Socket server for local IPC and reverse proxy communication.
#[cfg(all(unix, not(feature = "compio")))]
pub mod server_unix;

/// PROXY protocol v1/v2 parser for load balancer integration.
#[cfg(not(feature = "compio"))]
pub mod proxy_protocol;

/// Bind a TCP listener for `addr`, asking interactively to increment the port
/// if it is already in use.
///
/// This helper is primarily intended for local development and example binaries.
#[cfg(not(feature = "compio"))]
pub async fn bind_with_port_fallback(addr: &str) -> io::Result<tokio::net::TcpListener> {
  let mut socket_addr =
    SocketAddr::from_str(addr).map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?;
  let start_port = socket_addr.port();

  loop {
    let addr_str = socket_addr.to_string();
    match tokio::net::TcpListener::bind(&addr_str).await {
      Ok(listener) => {
        if socket_addr.port() != start_port {
          println!(
            "Port {} was in use, starting on {} instead",
            start_port,
            socket_addr.port()
          );
        }
        return Ok(listener);
      }
      Err(err) if err.kind() == ErrorKind::AddrInUse => {
        let next_port = socket_addr.port().saturating_add(1);
        if !ask_to_use_next_port(socket_addr.port(), next_port)? {
          return Err(err);
        }
        socket_addr.set_port(next_port);
      }
      Err(err) => return Err(err),
    }
  }
}

/// Bind a TCP listener for `addr`, asking interactively to increment the port
/// if it is already in use (compio version).
#[cfg(feature = "compio")]
pub async fn bind_with_port_fallback(addr: &str) -> io::Result<compio::net::TcpListener> {
  let mut socket_addr =
    SocketAddr::from_str(addr).map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?;
  let start_port = socket_addr.port();

  loop {
    let addr_str = socket_addr.to_string();
    match compio::net::TcpListener::bind(&addr_str).await {
      Ok(listener) => {
        if socket_addr.port() != start_port {
          println!(
            "Port {} was in use, starting on {} instead",
            start_port,
            socket_addr.port()
          );
        }
        return Ok(listener);
      }
      Err(err) if err.kind() == ErrorKind::AddrInUse => {
        let next_port = socket_addr.port().saturating_add(1);
        if !ask_to_use_next_port(socket_addr.port(), next_port)? {
          return Err(err);
        }
        socket_addr.set_port(next_port);
      }
      Err(err) => return Err(err),
    }
  }
}

fn ask_to_use_next_port(current: u16, next: u16) -> io::Result<bool> {
  loop {
    print!(
      "Port {} is already in use. Start on {} instead? [Y/n]: ",
      current, next
    );
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();

    if trimmed.is_empty()
      || trimmed.eq_ignore_ascii_case("y")
      || trimmed.eq_ignore_ascii_case("yes")
    {
      return Ok(true);
    }

    if trimmed.eq_ignore_ascii_case("n") || trimmed.eq_ignore_ascii_case("no") {
      return Ok(false);
    }

    println!("Please answer 'y' or 'n'.");
  }
}
