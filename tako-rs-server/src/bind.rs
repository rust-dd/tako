//! Interactive port-fallback TCP binding for local development.

use std::io::ErrorKind;
use std::io::Write;
use std::io::{self};
use std::net::SocketAddr;
use std::str::FromStr;

/// Binds `addr`, asking on stdin whether to try the next port while the port is in use; meant for
/// local development and examples.
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
        // There is no port after 65535, so report the original error.
        if curr_port == u16::MAX {
          return Err(err);
        }
        let next_port = curr_port + 1;
        // The stdin prompt blocks, so it must not run on a runtime worker.
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

/// Binds `addr`, asking on stdin whether to try the next port while the port is in use; meant for
/// local development and examples.
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
        // There is no port after 65535, so report the original error.
        if curr_port == u16::MAX {
          return Err(err);
        }
        let next_port = curr_port + 1;
        // The stdin prompt blocks, so it must not run on the runtime thread.
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
