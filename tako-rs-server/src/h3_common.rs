//! HTTP/3 pieces shared by the Tokio (quinn) and compio (`compio-quic`)
//! servers: QUIC transport limits and the request/response bridge.
//!
//! Both runtimes drive the same `quinn-proto` state machine, so one
//! `TransportConfig` builder serves both, and the bridge is generic over the
//! `h3` QUIC stream traits.

pub(crate) mod config;
pub(crate) mod request;
