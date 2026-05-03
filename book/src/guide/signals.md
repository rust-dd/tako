# Signals

> **Status:** scaffold.

In-process pub/sub for framework-internal events. Signal IDs are
documented in [API stability](../reference/stability.md) — operators
write dashboards and alerts against them, so renaming an ID is a
major-version event.

Per-request and connection-level signals are emitted from a single
site (`Router::dispatch` plus
`tako_core::signals::transport::{emit_server_started,
emit_connection_opened, emit_connection_closed}`), so every transport
gets the same signal payload for free.

The `signals::bus::SignalBus` async trait carries cluster-wide
forwarding. The default `LocalBus` is a no-op; companion crates can
implement Redis pub/sub, NATS, or Kafka backends.
