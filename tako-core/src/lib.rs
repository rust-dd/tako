#![cfg_attr(docsrs, feature(doc_cfg))]

//! Internal core for the Tako framework.
//!
//! This crate hosts the framework primitives shared across all Tako sub-crates:
//! routing, request/response types, body, middleware/plugin traits, extractor
//! traits, state, signals, queue, and a few cross-cutting features such as
//! `graphql`, `grpc`, and `openapi` that interact tightly with the router.
//!
//! Concrete extractors live in `tako-extractors`, server bootstrap code in
//! `tako-server`, streaming/upgrade transports in `tako-streams`, and concrete
//! middleware/plugin implementations in `tako-plugins`. Users should depend on
//! the `tako-rs` umbrella crate, which re-exports everything under the original
//! `tako::*` paths.

/// HTTP request and response body handling utilities.
pub mod body;

/// HTTP client implementation for making outbound requests.
#[cfg(all(feature = "client", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub mod client;

/// Configuration loading from environment variables.
pub mod config;

/// Request data extraction trait + the two extractors (`json`, `params`)
/// whose internal types are referenced by the router and route.
pub mod extractors;

/// Request handler traits and implementations.
pub mod handler;

/// Middleware trait + `Next` execution chain.
pub mod middleware;

/// Plugin system trait (`TakoPlugin`).
#[cfg(feature = "plugins")]
#[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
pub mod plugins;

/// Response generation utilities and traits.
pub mod responder;

/// RFC 7807 / RFC 9457 `application/problem+json` error responses.
pub mod problem;

/// Unified per-connection metadata extension shared by every transport.
pub mod conn_info;

/// Shared TLS certificate / key PEM loading helpers.
#[cfg(any(feature = "tls", feature = "http3", feature = "client"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "tls", feature = "http3", feature = "client"))))]
pub mod tls;

/// Redirection utilities for handling HTTP redirects.
pub mod redirect;

/// Route definition and matching logic.
pub mod route;

/// Request routing and dispatch functionality.
pub mod router;

/// In-memory background job queue with retry, delayed jobs, and dead letter support.
pub mod queue;

/// Application state management and dependency injection.
pub mod state;

/// Per-router typed state container (instance-scoped, complements `state`).
pub mod router_state;

#[cfg(feature = "signals")]
/// In-process signal arbiter for custom events.
pub mod signals;

/// Distributed tracing integration for observability.
#[cfg(feature = "tako-tracing")]
#[cfg_attr(docsrs, doc(cfg(feature = "tako-tracing")))]
pub mod tracing;

/// Core type definitions used throughout the framework.
pub mod types;

/// GraphQL support (request extractors, responses, and subscriptions).
#[cfg(feature = "async-graphql")]
#[cfg_attr(docsrs, doc(cfg(feature = "async-graphql")))]
pub mod graphql;

/// GraphiQL UI helpers.
#[cfg(feature = "graphiql")]
#[cfg_attr(docsrs, doc(cfg(feature = "graphiql")))]
pub mod graphiql;

/// OpenAPI documentation generation integrations (utoipa, vespera).
#[cfg(any(feature = "utoipa", feature = "vespera"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
pub mod openapi;

/// gRPC support for unary RPCs with protobuf serialization.
#[cfg(feature = "grpc")]
#[cfg_attr(docsrs, doc(cfg(feature = "grpc")))]
pub mod grpc;

pub use bytes::Bytes;
pub use http::Method;
pub use http::StatusCode;
pub use http::header;
pub use http_body_util::Full;
pub use responder::NOT_FOUND;
