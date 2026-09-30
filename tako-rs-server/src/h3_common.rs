//! HTTP/3 pieces shared by the Tokio (quinn) and compio (`compio-quic`)
//! servers: QUIC transport limits, the request/response bridge, and the
//! WebTransport session hand-off.
//!
//! Both runtimes drive the same `quinn-proto` state machine, so one
//! `TransportConfig` builder serves both, and the bridge is generic over the
//! `h3` QUIC stream traits.

pub(crate) mod config;
pub(crate) mod connection;
pub(crate) mod request;
pub(crate) mod sessions;
