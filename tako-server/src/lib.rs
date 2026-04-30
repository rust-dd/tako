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
use std::time::Duration;

/// Production-readiness knobs shared by every Tako server transport.
///
/// `Default` mirrors the historical hardcoded values (30 s drain, 30 s header
/// read, 100 H2 streams, …) so existing call sites keep their behavior. Pass
/// a populated `ServerConfig` to `*_with_config` entry points to override
/// individual knobs.
#[derive(Debug, Clone)]
pub struct ServerConfig {
  /// Maximum time the coordinator waits for in-flight connections to finish
  /// after a shutdown signal. After this elapses, remaining tasks are aborted.
  pub drain_timeout: Duration,
  /// Maximum time hyper waits for the request line + headers to arrive.
  /// `None` disables the timeout (the previous behavior).
  pub header_read_timeout: Option<Duration>,
  /// HTTP/1 keep-alive (default `true`).
  pub keep_alive: bool,
  /// HTTP/1 keep-alive idle timeout (Hyper default applies if `None`).
  pub keep_alive_timeout: Option<Duration>,
  /// HTTP/2 `SETTINGS_MAX_CONCURRENT_STREAMS` cap.
  pub h2_max_concurrent_streams: u32,
  /// HTTP/2 `SETTINGS_MAX_HEADER_LIST_SIZE` cap (bytes).
  pub h2_max_header_list_size: u32,
  /// HTTP/2 send-buffer cap per stream (bytes).
  pub h2_max_send_buf_size: usize,
  /// HTTP/2 pending-accept RST_STREAM cap (CVE-2023-44487 mitigation).
  pub h2_max_pending_accept_reset_streams: usize,
  /// HTTP/2 keep-alive ping interval. `None` disables.
  pub h2_keep_alive_interval: Option<Duration>,
  /// Optional ceiling on concurrent in-flight connections. Enforced via a
  /// semaphore in the accept loop; `None` disables.
  pub max_connections: Option<usize>,
  /// Read deadline applied before the PROXY protocol header is parsed.
  pub proxy_read_timeout: Duration,
  /// Backoff schedule for `accept()` errors (typically EMFILE/ENFILE).
  pub accept_backoff: AcceptBackoff,
}

impl Default for ServerConfig {
  fn default() -> Self {
    Self {
      drain_timeout: Duration::from_secs(30),
      header_read_timeout: Some(Duration::from_secs(30)),
      keep_alive: true,
      keep_alive_timeout: None,
      h2_max_concurrent_streams: 100,
      h2_max_header_list_size: 16 * 1024,
      h2_max_send_buf_size: 1024 * 1024,
      h2_max_pending_accept_reset_streams: 50,
      h2_keep_alive_interval: None,
      max_connections: None,
      proxy_read_timeout: Duration::from_secs(10),
      accept_backoff: AcceptBackoff::new(),
    }
  }
}

/// Exponential backoff state for `listener.accept()` retry loops.
///
/// Accept errors (typically `EMFILE`/`ENFILE` when the process has run out of
/// file descriptors, or transient `ConnectionAborted` under load) are not fatal
/// to the listener. Servers should log, sleep, and re-poll. Use [`AcceptBackoff`]
/// to keep the sleep schedule consistent across transports without duplicating
/// the constants in every `serve_*` implementation.
#[derive(Debug, Clone, Copy)]
pub struct AcceptBackoff {
  current: Duration,
  max: Duration,
}

impl Default for AcceptBackoff {
  fn default() -> Self {
    Self::new()
  }
}

impl AcceptBackoff {
  /// Construct with the default 5 ms → 1 s schedule.
  #[must_use]
  pub const fn new() -> Self {
    Self {
      current: Duration::from_millis(5),
      max: Duration::from_secs(1),
    }
  }

  /// Reset the schedule after a successful accept.
  #[inline]
  pub fn reset(&mut self) {
    self.current = Duration::from_millis(5);
  }

  /// Sleep for the current backoff and double it (capped at `max`).
  /// Use the tokio `sleep` so this is cooperative on the runtime that runs
  /// the accept loop.
  pub async fn sleep_and_grow(&mut self) {
    let d = self.current;
    self.current = (self.current * 2).min(self.max);
    tokio::time::sleep(d).await;
  }
}

#[cfg(not(feature = "compio"))]
mod server;

mod builder;
#[cfg(not(feature = "compio"))]
pub use builder::{Server, ServerBuilder};
#[cfg(feature = "compio")]
pub use builder::{CompioServer, CompioServerBuilder};
pub use builder::{ServerHandle, TlsCert, either};

#[cfg(not(feature = "compio"))]
pub use server::serve;
#[cfg(not(feature = "compio"))]
pub use server::serve_with_config;
#[cfg(not(feature = "compio"))]
pub use server::serve_with_shutdown;
#[cfg(not(feature = "compio"))]
pub use server::serve_with_shutdown_and_config;

#[cfg(feature = "compio")]
#[cfg_attr(docsrs, doc(cfg(feature = "compio")))]
pub mod server_compio;
#[cfg(feature = "compio")]
pub use server_compio::serve;
#[cfg(feature = "compio")]
pub use server_compio::serve_with_config;
#[cfg(feature = "compio")]
pub use server_compio::serve_with_shutdown;
#[cfg(feature = "compio")]
pub use server_compio::serve_with_shutdown_and_config;

/// TLS/SSL server implementation for secure connections.
#[cfg(all(not(feature = "compio-tls"), feature = "tls"))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub mod server_tls;
#[cfg(all(not(feature = "compio"), feature = "tls"))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use server_tls::serve_tls;
#[cfg(all(not(feature = "compio"), feature = "tls"))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use server_tls::serve_tls_with_config;
#[cfg(all(not(feature = "compio"), feature = "tls"))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use server_tls::serve_tls_with_shutdown;
#[cfg(all(not(feature = "compio"), feature = "tls"))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use server_tls::serve_tls_with_shutdown_and_config;

#[cfg(feature = "compio-tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "compio-tls")))]
pub mod server_tls_compio;
#[cfg(feature = "compio-tls")]
pub use server_tls_compio::serve_tls;
#[cfg(feature = "compio-tls")]
pub use server_tls_compio::serve_tls_with_config;
#[cfg(feature = "compio-tls")]
pub use server_tls_compio::serve_tls_with_shutdown;
#[cfg(feature = "compio-tls")]
pub use server_tls_compio::serve_tls_with_shutdown_and_config;

/// HTTP/3 server implementation using QUIC transport.
#[cfg(all(feature = "http3", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub mod server_h3;
#[cfg(all(feature = "http3", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub use server_h3::serve_h3;
#[cfg(all(feature = "http3", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub use server_h3::serve_h3_with_config;
#[cfg(all(feature = "http3", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub use server_h3::serve_h3_with_shutdown;
#[cfg(all(feature = "http3", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub use server_h3::serve_h3_with_shutdown_and_config;

/// Raw TCP server for handling arbitrary TCP connections.
pub mod server_tcp;

/// HTTP/2 cleartext (h2c, prior knowledge) server.
#[cfg(all(feature = "http2", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http2")))]
pub mod server_h2c;
#[cfg(all(feature = "http2", not(feature = "compio")))]
pub use server_h2c::serve_h2c;
#[cfg(all(feature = "http2", not(feature = "compio")))]
pub use server_h2c::serve_h2c_with_config;
#[cfg(all(feature = "http2", not(feature = "compio")))]
pub use server_h2c::serve_h2c_with_shutdown;
#[cfg(all(feature = "http2", not(feature = "compio")))]
pub use server_h2c::serve_h2c_with_shutdown_and_config;

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
