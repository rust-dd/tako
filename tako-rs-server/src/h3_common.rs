//! HTTP/3 code shared by the quinn and `compio-quic` servers, which drive the same `quinn-proto`
//! state machine: transport limits, the request bridge and the WebTransport hand-off.

pub(crate) mod config;
pub(crate) mod connection;
pub(crate) mod request;
pub(crate) mod sessions;
