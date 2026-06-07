//! Connection-lifecycle signal helpers used by every transport.
//!
//! `Router::dispatch` already emits the per-request `REQUEST_STARTED` /
//! `REQUEST_COMPLETED` signals automatically; these helpers cover the
//! connection-level events (`SERVER_STARTED`, `CONNECTION_OPENED`,
//! `CONNECTION_CLOSED`) that have no natural per-request hook. They keep the
//! emit boilerplate out of every transport file.

use super::arbiter::SignalArbiter;
use super::signal::Signal;
use super::signal::ids;

/// Emits the `server.started` signal with `addr` / `transport` / `tls` meta.
pub async fn emit_server_started(addr: &str, transport: &str, tls: bool) {
  SignalArbiter::emit_app(
    Signal::with_capacity(ids::SERVER_STARTED, 3)
      .meta("addr", addr)
      .meta("transport", transport)
      .meta("tls", if tls { "true" } else { "false" }),
  )
  .await;
}

/// Emits the `connection.opened` signal with `remote_addr` / `tls` / optional `protocol`.
pub async fn emit_connection_opened(remote_addr: &str, tls: bool, protocol: Option<&str>) {
  let mut sig = Signal::with_capacity(ids::CONNECTION_OPENED, 3)
    .meta("remote_addr", remote_addr)
    .meta("tls", if tls { "true" } else { "false" });
  if let Some(p) = protocol {
    sig = sig.meta("protocol", p);
  }
  SignalArbiter::emit_app(sig).await;
}

/// Emits the `connection.closed` signal with `remote_addr` / `tls` / optional `protocol`.
pub async fn emit_connection_closed(remote_addr: &str, tls: bool, protocol: Option<&str>) {
  let mut sig = Signal::with_capacity(ids::CONNECTION_CLOSED, 3)
    .meta("remote_addr", remote_addr)
    .meta("tls", if tls { "true" } else { "false" });
  if let Some(p) = protocol {
    sig = sig.meta("protocol", p);
  }
  SignalArbiter::emit_app(sig).await;
}
