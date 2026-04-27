#![cfg_attr(docsrs, feature(doc_cfg))]

//! Streaming and upgrade transports for the Tako framework.
//!
//! WebSocket, Server-Sent Events, file streaming, static file serving, and
//! WebTransport implementations. Re-exported under the original `tako::*` paths
//! via the umbrella crate.

/// Server-Sent Events (SSE) support for real-time communication.
pub mod sse;

/// File streaming utilities for serving files.
#[cfg(feature = "file-stream")]
#[cfg_attr(docsrs, doc(cfg(feature = "file-stream")))]
pub mod file_stream;

/// Static file serving utilities.
pub mod r#static;

/// WebSocket connection handling and message processing.
#[cfg(not(feature = "compio"))]
pub mod ws;

/// WebSocket connection handling for compio runtime.
#[cfg(feature = "compio-ws")]
#[cfg_attr(docsrs, doc(cfg(feature = "compio-ws")))]
pub mod ws_compio;

/// WebTransport server support over QUIC.
#[cfg(all(feature = "webtransport", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "webtransport")))]
pub mod webtransport;
