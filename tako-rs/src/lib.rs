#![cfg_attr(docsrs, feature(doc_cfg))]

//! A multi-transport Rust framework for modern network services.
//!
//! Tako is built for services that go beyond plain HTTP. It gives you one
//! cohesive model for routing, extraction, middleware, streaming, observability,
//! and graceful shutdown across several protocols and transport layers.
//!
//! This umbrella crate stitches together the workspace sub-crates:
//!
//! - `tako-core` — routing, handlers, middleware and plugin traits, body and
//!   request types, state, signals, queue, plus GraphQL, gRPC and OpenAPI
//!   helpers
//! - `tako-extractors` — concrete request extractors (cookies, form, query,
//!   path, JWT, multipart, simdjson, …)
//! - `tako-server` — HTTP/1, TLS, HTTP/3, raw TCP / UDP / Unix, PROXY protocol,
//!   plus the compio variants
//! - `tako-streams` — WebSocket, SSE, file streaming, static file serving,
//!   WebTransport
//! - `tako-plugins` — built-in middleware (auth, CSRF, sessions, …) and
//!   plugins (CORS, compression, rate limiting, idempotency, metrics)
//!
//! All public types stay reachable at the original `tako::*` paths.

pub use tako_core::Bytes;
pub use tako_core::Full;
pub use tako_core::Method;
pub use tako_core::NOT_FOUND;
pub use tako_core::StatusCode;
pub use tako_core::header;

pub use tako_macros::delete;
pub use tako_macros::get;
pub use tako_macros::patch;
pub use tako_macros::post;
pub use tako_macros::put;
pub use tako_macros::route;

/// Implementation details for tako's proc macros. Not part of the stable
/// API — relied on only by macro-generated code.
#[doc(hidden)]
pub mod __private {
  pub use linkme;
}

pub use tako_core::body;
pub use tako_core::config;
pub use tako_core::conn_info;
pub use tako_core::problem;
pub use tako_core::router_state;
pub use tako_core::queue;
pub use tako_core::redirect;
pub use tako_core::responder;
pub use tako_core::route;
pub use tako_core::router;
pub use tako_core::state;
pub use tako_core::types;

#[cfg(all(feature = "client", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub use tako_core::client;

#[cfg(feature = "signals")]
#[cfg_attr(docsrs, doc(cfg(feature = "signals")))]
pub use tako_core::signals;

#[cfg(feature = "tako-tracing")]
#[cfg_attr(docsrs, doc(cfg(feature = "tako-tracing")))]
pub use tako_core::tracing;

#[cfg(feature = "async-graphql")]
#[cfg_attr(docsrs, doc(cfg(feature = "async-graphql")))]
pub use tako_core::graphql;

#[cfg(feature = "graphiql")]
#[cfg_attr(docsrs, doc(cfg(feature = "graphiql")))]
pub use tako_core::graphiql;

#[cfg(any(feature = "utoipa", feature = "vespera"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
pub use tako_core::openapi;

#[cfg(feature = "grpc")]
#[cfg_attr(docsrs, doc(cfg(feature = "grpc")))]
pub use tako_core::grpc;

pub use tako_streams::r#static;
pub use tako_streams::sse;

#[cfg(feature = "file-stream")]
#[cfg_attr(docsrs, doc(cfg(feature = "file-stream")))]
pub use tako_streams::file_stream;

#[cfg(not(any(feature = "compio", feature = "compio-ws")))]
pub use tako_streams::ws;

#[cfg(feature = "compio-ws")]
#[cfg_attr(docsrs, doc(cfg(feature = "compio-ws")))]
pub use tako_streams::ws_compio;

#[cfg(all(feature = "webtransport", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "webtransport")))]
pub use tako_streams::webtransport;

pub use tako_server::server_tcp;
pub use tako_server::server_udp;

#[cfg(all(feature = "http2", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http2")))]
pub use tako_server::server_h2c;
#[cfg(all(feature = "http2", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http2")))]
pub use tako_server::serve_h2c;
#[cfg(all(feature = "http2", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http2")))]
pub use tako_server::serve_h2c_with_config;
#[cfg(all(feature = "http2", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http2")))]
pub use tako_server::serve_h2c_with_shutdown;
#[cfg(all(feature = "http2", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http2")))]
pub use tako_server::serve_h2c_with_shutdown_and_config;

#[cfg(all(unix, not(any(feature = "compio", feature = "compio-tls", feature = "compio-ws"))))]
pub use tako_server::server_unix;

#[cfg(not(any(feature = "compio", feature = "compio-tls", feature = "compio-ws")))]
pub use tako_server::proxy_protocol;

#[cfg(feature = "compio")]
#[cfg_attr(docsrs, doc(cfg(feature = "compio")))]
pub use tako_server::server_compio;

#[cfg(all(not(feature = "compio-tls"), feature = "tls"))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use tako_server::server_tls;

#[cfg(feature = "compio-tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "compio-tls")))]
pub use tako_server::server_tls_compio;

#[cfg(all(feature = "http3", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub use tako_server::server_h3;

pub use tako_server::AcceptBackoff;
pub use tako_server::ServerConfig;
pub use tako_server::{ServerHandle, TlsCert};
#[cfg(not(feature = "compio"))]
pub use tako_server::{Server, ServerBuilder};
#[cfg(feature = "compio")]
pub use tako_server::{CompioServer, CompioServerBuilder};
pub use tako_server::bind_with_port_fallback;
pub use tako_server::serve;
pub use tako_server::serve_with_shutdown;
#[cfg(not(feature = "compio"))]
pub use tako_server::serve_with_config;
#[cfg(not(feature = "compio"))]
pub use tako_server::serve_with_shutdown_and_config;

#[cfg(all(feature = "http3", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub use tako_server::serve_h3;
#[cfg(all(feature = "http3", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub use tako_server::serve_h3_with_config;
#[cfg(all(feature = "http3", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub use tako_server::serve_h3_with_shutdown;
#[cfg(all(feature = "http3", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub use tako_server::serve_h3_with_shutdown_and_config;

#[cfg(any(
  all(feature = "tls", not(any(feature = "compio", feature = "compio-tls", feature = "compio-ws"))),
  feature = "compio-tls"
))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "tls", feature = "compio-tls"))))]
pub use tako_server::serve_tls;
#[cfg(any(
  all(feature = "tls", not(any(feature = "compio", feature = "compio-tls", feature = "compio-ws"))),
  feature = "compio-tls"
))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "tls", feature = "compio-tls"))))]
pub use tako_server::serve_tls_with_shutdown;
#[cfg(all(feature = "tls", not(any(feature = "compio", feature = "compio-tls", feature = "compio-ws"))))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use tako_server::serve_tls_with_config;
#[cfg(all(feature = "tls", not(any(feature = "compio", feature = "compio-tls", feature = "compio-ws"))))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use tako_server::serve_tls_with_shutdown_and_config;

/// Request data extraction utilities.
pub mod extractors {
  pub use tako_core::extractors::FromRequest;
  pub use tako_core::extractors::FromRequestParts;
  #[doc(hidden)]
  pub use tako_core::extractors::is_json_content_type;
  pub use tako_core::extractors::json;
  pub use tako_core::extractors::params;
  pub use tako_core::extractors::range;
  pub use tako_core::extractors::typed_params;

  pub use tako_extractors::acc_lang;
  pub use tako_extractors::accept;
  pub use tako_extractors::basic;
  pub use tako_extractors::bearer;
  pub use tako_extractors::bytes;
  pub use tako_extractors::cookie_jar;
  pub use tako_extractors::cookie_key_expansion;
  pub use tako_extractors::cookie_private;
  pub use tako_extractors::cookie_signed;
  pub use tako_extractors::form;
  pub use tako_extractors::header_map;
  pub use tako_extractors::ipaddr;
  pub use tako_extractors::jwt;
  pub use tako_extractors::path;
  pub use tako_extractors::query;
  pub use tako_extractors::state;

  #[cfg(feature = "multipart")]
  #[cfg_attr(docsrs, doc(cfg(feature = "multipart")))]
  pub use tako_extractors::multipart;

  #[cfg(feature = "protobuf")]
  #[cfg_attr(docsrs, doc(cfg(feature = "protobuf")))]
  pub use tako_extractors::protobuf;

  #[cfg(feature = "simd")]
  #[cfg_attr(docsrs, doc(cfg(feature = "simd")))]
  pub use tako_extractors::simdjson;
}

/// Middleware for processing requests and responses in a pipeline.
pub mod middleware {
  pub use tako_core::middleware::IntoMiddleware;
  pub use tako_core::middleware::Next;

  pub use tako_plugins::middleware::api_key_auth;
  pub use tako_plugins::middleware::basic_auth;
  pub use tako_plugins::middleware::bearer_auth;
  pub use tako_plugins::middleware::body_limit;
  pub use tako_plugins::middleware::csrf;
  pub use tako_plugins::middleware::jwt_auth;
  pub use tako_plugins::middleware::request_id;
  pub use tako_plugins::middleware::security_headers;
  pub use tako_plugins::middleware::session;
  pub use tako_plugins::middleware::upload_progress;
}

#[cfg(feature = "plugins")]
#[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
pub mod plugins {
  pub use tako_core::plugins::TakoPlugin;

  pub use tako_plugins::plugins::compression;
  pub use tako_plugins::plugins::cors;
  pub use tako_plugins::plugins::idempotency;
  pub use tako_plugins::plugins::rate_limiter;

  #[cfg(any(feature = "metrics-prometheus", feature = "metrics-opentelemetry"))]
  #[cfg_attr(
    docsrs,
    doc(cfg(any(feature = "metrics-prometheus", feature = "metrics-opentelemetry")))
  )]
  pub use tako_plugins::plugins::metrics;
}

#[cfg(feature = "zero-copy-extractors")]
#[cfg_attr(docsrs, doc(cfg(feature = "zero-copy-extractors")))]
pub use tako_extractors::zero_copy_extractors;

#[cfg(feature = "per-thread")]
#[cfg_attr(docsrs, doc(cfg(feature = "per-thread")))]
pub use tako_server_pt::PerThreadConfig;
#[cfg(feature = "per-thread")]
#[cfg_attr(docsrs, doc(cfg(feature = "per-thread")))]
pub use tako_server_pt::serve_per_thread;

#[cfg(feature = "per-thread-compio")]
#[cfg_attr(docsrs, doc(cfg(feature = "per-thread-compio")))]
pub use tako_server_pt::serve_per_thread_compio;

#[cfg(feature = "jemalloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "jemalloc")))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
