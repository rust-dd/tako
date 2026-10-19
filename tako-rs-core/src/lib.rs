#![cfg_attr(docsrs, feature(doc_cfg))]
// Extractor impls spell out `-> impl Future + Send`; an `async fn` would not promise `Send`.
#![allow(clippy::manual_async_fn)]

//! Routing, handler, extractor and responder primitives shared by the Tako crates.
//!
//! Depend on the `tako-rs` umbrella crate instead; it re-exports these under `tako::*`.

pub mod body;
pub mod config;
pub mod conn_info;
pub mod extractors;
pub mod handler;
pub mod middleware;
pub mod problem;
pub mod queue;
pub mod redirect;
pub mod responder;
pub mod route;
pub mod router;
pub mod router_state;
pub mod state;
pub mod types;

mod recycle;

#[doc(hidden)]
pub mod server_support;

#[cfg(feature = "plugins")]
#[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
pub mod plugins;

#[cfg(any(feature = "tls", feature = "http3"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "tls", feature = "http3"))))]
pub mod tls;

#[cfg(feature = "signals")]
pub mod signals;

#[cfg(feature = "tako-tracing")]
#[cfg_attr(docsrs, doc(cfg(feature = "tako-tracing")))]
pub mod tracing;

#[cfg(feature = "async-graphql")]
#[cfg_attr(docsrs, doc(cfg(feature = "async-graphql")))]
pub mod graphql;

#[cfg(feature = "graphiql")]
#[cfg_attr(docsrs, doc(cfg(feature = "graphiql")))]
pub mod graphiql;

#[cfg(any(feature = "utoipa", feature = "vespera"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
pub mod openapi;

#[cfg(feature = "grpc")]
#[cfg_attr(docsrs, doc(cfg(feature = "grpc")))]
pub mod grpc;

#[cfg(feature = "ws")]
#[cfg_attr(docsrs, doc(cfg(feature = "ws")))]
pub mod ws;

pub use bytes::Bytes;
pub use http::Method;
pub use http::StatusCode;
pub use http::header;
pub use http_body_util::Full;
pub use responder::NOT_FOUND;
