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
#[cfg(all(feature = "client", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub use tako_core::client;
pub use tako_core::config;
pub use tako_core::conn_info;
#[cfg(feature = "graphiql")]
#[cfg_attr(docsrs, doc(cfg(feature = "graphiql")))]
pub use tako_core::graphiql;
#[cfg(feature = "async-graphql")]
#[cfg_attr(docsrs, doc(cfg(feature = "async-graphql")))]
pub use tako_core::graphql;
#[cfg(feature = "grpc")]
#[cfg_attr(docsrs, doc(cfg(feature = "grpc")))]
pub use tako_core::grpc;
#[cfg(any(feature = "utoipa", feature = "vespera"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
pub use tako_core::openapi;
pub use tako_core::problem;
pub use tako_core::queue;
pub use tako_core::redirect;
pub use tako_core::responder;
pub use tako_core::route;
pub use tako_core::router;
pub use tako_core::router_state;
#[cfg(feature = "signals")]
#[cfg_attr(docsrs, doc(cfg(feature = "signals")))]
pub use tako_core::signals;
pub use tako_core::state;
#[cfg(feature = "tako-tracing")]
#[cfg_attr(docsrs, doc(cfg(feature = "tako-tracing")))]
pub use tako_core::tracing;
pub use tako_core::types;
pub use tako_server::AcceptBackoff;
#[cfg(feature = "compio")]
pub use tako_server::CompioServer;
#[cfg(feature = "compio")]
pub use tako_server::CompioServerBuilder;
#[cfg(not(feature = "compio"))]
pub use tako_server::Server;
#[cfg(not(feature = "compio"))]
pub use tako_server::ServerBuilder;
pub use tako_server::ServerConfig;
pub use tako_server::ServerHandle;
pub use tako_server::TlsCert;
pub use tako_server::bind_with_port_fallback;
#[cfg(not(any(feature = "compio", feature = "compio-tls", feature = "compio-ws")))]
pub use tako_server::proxy_protocol;
pub use tako_server::serve;
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
  all(
    feature = "tls",
    not(any(feature = "compio", feature = "compio-tls", feature = "compio-ws"))
  ),
  feature = "compio-tls"
))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "tls", feature = "compio-tls"))))]
pub use tako_server::serve_tls;
#[cfg(all(
  feature = "tls",
  not(any(feature = "compio", feature = "compio-tls", feature = "compio-ws"))
))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use tako_server::serve_tls_with_config;
#[cfg(any(
  all(
    feature = "tls",
    not(any(feature = "compio", feature = "compio-tls", feature = "compio-ws"))
  ),
  feature = "compio-tls"
))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "tls", feature = "compio-tls"))))]
pub use tako_server::serve_tls_with_shutdown;
#[cfg(all(
  feature = "tls",
  not(any(feature = "compio", feature = "compio-tls", feature = "compio-ws"))
))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use tako_server::serve_tls_with_shutdown_and_config;
#[cfg(not(feature = "compio"))]
pub use tako_server::serve_with_config;
pub use tako_server::serve_with_shutdown;
#[cfg(not(feature = "compio"))]
pub use tako_server::serve_with_shutdown_and_config;
#[cfg(feature = "compio")]
#[cfg_attr(docsrs, doc(cfg(feature = "compio")))]
pub use tako_server::server_compio;
#[cfg(all(feature = "http2", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http2")))]
pub use tako_server::server_h2c;
#[cfg(all(feature = "http3", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub use tako_server::server_h3;
pub use tako_server::server_tcp;
#[cfg(all(not(feature = "compio-tls"), feature = "tls"))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use tako_server::server_tls;
#[cfg(feature = "compio-tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "compio-tls")))]
pub use tako_server::server_tls_compio;
pub use tako_server::server_udp;
#[cfg(all(
  unix,
  not(any(feature = "compio", feature = "compio-tls", feature = "compio-ws"))
))]
pub use tako_server::server_unix;
#[cfg(feature = "file-stream")]
#[cfg_attr(docsrs, doc(cfg(feature = "file-stream")))]
pub use tako_streams::file_stream;
pub use tako_streams::sse;
pub use tako_streams::r#static;
#[cfg(all(feature = "webtransport", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "webtransport")))]
pub use tako_streams::webtransport;
#[cfg(not(any(feature = "compio", feature = "compio-ws")))]
pub use tako_streams::ws;
#[cfg(feature = "compio-ws")]
#[cfg_attr(docsrs, doc(cfg(feature = "compio-ws")))]
pub use tako_streams::ws_compio;

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
  #[cfg(feature = "multipart")]
  #[cfg_attr(docsrs, doc(cfg(feature = "multipart")))]
  pub use tako_extractors::multipart;
  pub use tako_extractors::path;
  #[cfg(feature = "protobuf")]
  #[cfg_attr(docsrs, doc(cfg(feature = "protobuf")))]
  pub use tako_extractors::protobuf;
  pub use tako_extractors::query;
  pub use tako_extractors::query_multi;
  pub use tako_extractors::connect_info;
  pub use tako_extractors::content_length_limit;
  pub use tako_extractors::extension;
  pub use tako_extractors::matched_path;
  pub use tako_extractors::uri_parts;
  #[cfg(feature = "typed-header")]
  #[cfg_attr(docsrs, doc(cfg(feature = "typed-header")))]
  pub use tako_extractors::typed_header;
  #[cfg(any(feature = "validator", feature = "garde"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "validator", feature = "garde"))))]
  pub use tako_extractors::validate;
  #[cfg(feature = "simd")]
  #[cfg_attr(docsrs, doc(cfg(feature = "simd")))]
  pub use tako_extractors::simdjson;
  pub use tako_extractors::state;
}

/// Middleware for processing requests and responses in a pipeline.
pub mod middleware {
  pub use tako_core::middleware::IntoMiddleware;
  pub use tako_core::middleware::Next;
  pub use tako_plugins::middleware::access_log;
  pub use tako_plugins::middleware::api_key_auth;
  pub use tako_plugins::middleware::basic_auth;
  pub use tako_plugins::middleware::bearer_auth;
  pub use tako_plugins::middleware::body_limit;
  pub use tako_plugins::middleware::circuit_breaker;
  pub use tako_plugins::middleware::csrf;
  pub use tako_plugins::middleware::etag;
  pub use tako_plugins::middleware::healthcheck;
  #[cfg(feature = "hmac-signature")]
  #[cfg_attr(docsrs, doc(cfg(feature = "hmac-signature")))]
  pub use tako_plugins::middleware::hmac_signature;
  #[cfg(feature = "ip-filter")]
  #[cfg_attr(docsrs, doc(cfg(feature = "ip-filter")))]
  pub use tako_plugins::middleware::ip_filter;
  #[cfg(feature = "json-schema")]
  #[cfg_attr(docsrs, doc(cfg(feature = "json-schema")))]
  pub use tako_plugins::middleware::json_schema;
  pub use tako_plugins::middleware::jwt_auth;
  pub use tako_plugins::middleware::problem_json;
  pub use tako_plugins::middleware::request_id;
  pub use tako_plugins::middleware::security_headers;
  pub use tako_plugins::middleware::session;
  pub use tako_plugins::middleware::tenant;
  pub use tako_plugins::middleware::timeout;
  pub use tako_plugins::middleware::traceparent;
  pub use tako_plugins::middleware::upload_progress;
}

/// Pluggable backend traits for stateful middleware (sessions, rate limiting, …).
pub mod stores {
  pub use tako_plugins::stores::*;
}

#[cfg(feature = "plugins")]
#[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
pub mod plugins {
  pub use tako_core::plugins::TakoPlugin;
  pub use tako_plugins::plugins::compression;
  pub use tako_plugins::plugins::cors;
  pub use tako_plugins::plugins::idempotency;
  #[cfg(any(feature = "metrics-prometheus", feature = "metrics-opentelemetry"))]
  #[cfg_attr(
    docsrs,
    doc(cfg(any(feature = "metrics-prometheus", feature = "metrics-opentelemetry")))
  )]
  pub use tako_plugins::plugins::metrics;
  pub use tako_plugins::plugins::rate_limiter;
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
