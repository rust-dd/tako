//! Interactive port-fallback TCP binding helpers for local development.

use std::io::ErrorKind;
use std::io::Write;
use std::io::{self};
use std::net::SocketAddr;
use std::str::FromStr;

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
        let curr_port = socket_addr.port();
        // Cap at u16::MAX — `saturating_add(1)` on 65535 returns 65535, so
        // a naive loop would re-bind the same port forever if the user keeps
        // answering "Y". Surface the original AddrInUse error instead.
        if curr_port == u16::MAX {
          return Err(err);
        }
        let next_port = curr_port + 1;
        // Synchronous stdin read on a blocking pool — the previous call
        // ran the read inline and blocked the async runtime worker until
        // the user typed Enter.
        let proceed =
          tokio::task::spawn_blocking(move || ask_to_use_next_port(curr_port, next_port))
            .await
            .map_err(io::Error::other)??;
        if !proceed {
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
        let curr_port = socket_addr.port();
        // See the tokio variant: avoid infinite loop on port 65535.
        if curr_port == u16::MAX {
          return Err(err);
        }
        let next_port = curr_port + 1;
        // compio variant: dedicate a blocking-pool task for the stdin read
        // so the io_uring/IOCP reactor isn't held by the prompt.
        let proceed =
          compio::runtime::spawn_blocking(move || ask_to_use_next_port(curr_port, next_port))
            .await
            .map_err(|_| io::Error::other("compio spawn_blocking panicked"))??;
        if !proceed {
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
    print!("Port {current} is already in use. Start on {next} instead? [Y/n]: ");
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
