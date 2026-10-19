//! Server and connection lifecycle signals for transports to emit; the router emits the
//! request signals itself.

use super::arbiter::SignalArbiter;
use super::signal::Signal;
use super::signal::ids;

/// Emits `server.started` with `addr`, `transport` and `tls` metadata.
pub async fn emit_server_started(addr: &str, transport: &str, tls: bool) {
  SignalArbiter::emit_app(
    Signal::with_capacity(ids::SERVER_STARTED, 3)
      .meta("addr", addr)
      .meta("transport", transport)
      .meta("tls", if tls { "true" } else { "false" }),
  )
  .await;
}

/// Emits `server.stopped`; call it once a listener stopped accepting and its connections drained.
pub async fn emit_server_stopped(addr: &str, transport: &str, tls: bool) {
  SignalArbiter::emit_app(
    Signal::with_capacity(ids::SERVER_STOPPED, 3)
      .meta("addr", addr)
      .meta("transport", transport)
      .meta("tls", if tls { "true" } else { "false" }),
  )
  .await;
}

/// Emits `connection.opened` with `remote_addr`, `tls` and, if known, `protocol` metadata.
pub async fn emit_connection_opened(remote_addr: &str, tls: bool, protocol: Option<&str>) {
  let mut sig = Signal::with_capacity(ids::CONNECTION_OPENED, 3)
    .meta("remote_addr", remote_addr)
    .meta("tls", if tls { "true" } else { "false" });
  if let Some(p) = protocol {
    sig = sig.meta("protocol", p);
  }
  SignalArbiter::emit_app(sig).await;
}

/// Emits `connection.closed` with `remote_addr`, `tls` and, if known, `protocol` metadata.
pub async fn emit_connection_closed(remote_addr: &str, tls: bool, protocol: Option<&str>) {
  let mut sig = Signal::with_capacity(ids::CONNECTION_CLOSED, 3)
    .meta("remote_addr", remote_addr)
    .meta("tls", if tls { "true" } else { "false" });
  if let Some(p) = protocol {
    sig = sig.meta("protocol", p);
  }
  SignalArbiter::emit_app(sig).await;
}
