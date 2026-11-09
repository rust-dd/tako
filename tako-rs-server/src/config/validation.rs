use std::fmt;
use std::time::Instant;

use super::ServerConfig;

/// A server setting that cannot be used safely by the transport implementations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerConfigError {
  field: &'static str,
  reason: &'static str,
}

impl ServerConfigError {
  const fn new(field: &'static str, reason: &'static str) -> Self {
    Self { field, reason }
  }

  /// The name of the invalid [`ServerConfig`] field.
  pub const fn field(&self) -> &'static str {
    self.field
  }

  /// The constraint that the setting must satisfy.
  pub const fn reason(&self) -> &'static str {
    self.reason
  }
}

impl fmt::Display for ServerConfigError {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(formatter, "{} {}", self.field, self.reason)
  }
}

impl std::error::Error for ServerConfigError {}

impl From<ServerConfigError> for std::io::Error {
  fn from(error: ServerConfigError) -> Self {
    Self::new(std::io::ErrorKind::InvalidInput, error)
  }
}

impl ServerConfig {
  /// Checks limits before a server binds sockets, initializes plugins, or starts tasks.
  ///
  /// Zero shutdown and handshake deadlines remain valid immediate-expiry policies. A zero
  /// HTTP/3 idle timeout disables that timeout, as defined by QUIC. Protocol stream, header,
  /// and send-buffer limits may be zero to preserve explicit throttling policies.
  pub fn validate(&self) -> Result<(), ServerConfigError> {
    if let Some(limit) = self.max_connections
      && (limit == 0 || limit > tokio::sync::Semaphore::MAX_PERMITS)
    {
      return Err(ServerConfigError::new(
        "max_connections",
        "must be between 1 and tokio::sync::Semaphore::MAX_PERMITS",
      ));
    }
    if self.h2_max_send_buf_size > u32::MAX as usize {
      return Err(ServerConfigError::new(
        "h2_max_send_buf_size",
        "must be at most u32::MAX",
      ));
    }
    if let Some(timeout) = self.h3_max_idle_timeout
      && timeout.as_millis() > (1_u128 << 62) - 1
    {
      return Err(ServerConfigError::new(
        "h3_max_idle_timeout",
        "must fit in a QUIC variable-length integer of milliseconds",
      ));
    }
    let now = Instant::now();
    for (field, timeout) in [
      ("drain_timeout", Some(self.drain_timeout)),
      ("header_read_timeout", self.header_read_timeout),
      ("h2_keep_alive_interval", self.h2_keep_alive_interval),
      ("h3_max_idle_timeout", self.h3_max_idle_timeout),
      ("h3_goaway_grace", Some(self.h3_goaway_grace)),
      ("proxy_read_timeout", Some(self.proxy_read_timeout)),
      ("tls_handshake_timeout", Some(self.tls_handshake_timeout)),
    ] {
      if let Some(timeout) = timeout
        && now.checked_add(timeout).is_none()
      {
        return Err(ServerConfigError::new(
          field,
          "exceeds the monotonic clock's range",
        ));
      }
    }
    Ok(())
  }
}
