#![cfg_attr(docsrs, feature(doc_cfg))]

//! SSE, file streaming, static files and raw QUIC sessions, re-exported under `tako::*`.

mod file_io;

#[cfg(feature = "sse")]
#[cfg_attr(docsrs, doc(cfg(feature = "sse")))]
pub mod sse;

#[cfg(feature = "file-stream")]
#[cfg_attr(docsrs, doc(cfg(feature = "file-stream")))]
pub mod file_stream;

pub mod r#static;

#[cfg(all(feature = "webtransport", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "webtransport")))]
pub mod webtransport;
