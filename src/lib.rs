#![cfg_attr(docsrs, feature(doc_cfg))]

//! A lightweight, modular web framework for async applications.
//!
//! Tako focuses on ergonomics and composability. It provides routing, extractors,
//! responses, middleware, streaming, WebSockets/SSE, optional TLS, and integrations
//! like GraphQL — in a small, pragmatic package.
//!
//! # High-level features
//! - Macro-free routing with dynamic path params and TSR support
//! - Type-safe handlers with extractor-based arguments (Axum-like ergonomics)
//! - Simple `Responder` trait to return strings, tuples, or full responses
//! - Middleware pipeline (auth, body limits, etc.) and optional plugins (CORS, compression, rate limits)
//! - Streaming bodies, file serving, range requests, and SSE
//! - WebSocket upgrades and helpers
//! - Optional TLS (rustls) and HTTP/2 (feature)
//! - Optional GraphQL support (async-graphql) and GraphiQL UI
//!
//! # Compatibility
//! - Runtime: `tokio`
//! - HTTP: `hyper` 1.x
//!
//! # Quickstart
//!
//! ```rust
//! use tako::{Method, router::Router, responder::Responder, types::Request};
//!
//! async fn hello(_: Request) -> impl Responder { "Hello, World!" }
//!
//! let mut router = Router::new();
//! router.route(Method::GET, "/", hello);
//! ```
//!
//! # Key concepts
//! - [router::Router] manages routes, middleware and dispatch.
//! - [extractors] parse request data (headers, params, JSON, forms, etc.).
//! - [responder::Responder] converts return values into HTTP responses.
//! - [middleware] composes cross-cutting concerns.
//!
//! - Static file serving (module `static`) and [file_stream] provide static and streaming file responses.
//! - [ws] and [sse] enable real-time communication.
//! - [plugins] add CORS, compression, and rate limiting (feature: `plugins`).
//! - [graphql] and [graphiql] add GraphQL support (feature: `async-graphql` / `graphiql`).
//!
//! # Feature flags
//! - `client` — outbound HTTP clients over TCP/TLS
//! - `file-stream` — file streaming utilities
//! - `http2` — enable ALPN h2 in TLS server
//! - `jemalloc` — use jemalloc as global allocator
//! - `multipart` — multipart form-data extractors
//! - `plugins` — CORS, compression, rate limiting
//! - `protobuf` — protobuf extractors (prost)
//! - `simd` — SIMD JSON extractor (simd-json)
//! - `tls` — TLS server (rustls)
//! - `tako-tracing` — structured tracing subscriber
//! - `zstd` — Zstandard compression option within plugins::compression

/// HTTP request and response body handling utilities.
pub mod body;

/// HTTP client implementation for making outbound requests.
#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub mod client;

/// Request data extraction utilities for parsing query params, JSON, and more.
pub mod extractors;

/// File streaming utilities for serving files.
#[cfg(feature = "file-stream")]
#[cfg_attr(docsrs, doc(cfg(feature = "file-stream")))]
pub mod file_stream;

/// Request handler traits and implementations.
mod handler;

/// Middleware for processing requests and responses in a pipeline.
pub mod middleware;

/// Plugin system for extending framework functionality.
#[cfg(feature = "plugins")]
#[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
pub mod plugins;

/// Response generation utilities and traits.
pub mod responder;

/// Redirection utilities for handling HTTP redirects.
pub mod redirect;

/// Route definition and matching logic.
mod route;

/// Request routing and dispatch functionality.
pub mod router;

/// HTTP server implementation and configuration.
mod server;

/// Server-Sent Events (SSE) support for real-time communication.
pub mod sse;

/// Application state management and dependency injection.
pub mod state;

/// In-process signal arbiter for custom events.
pub mod signals;

/// Static file serving utilities.
pub mod r#static;

/// Distributed tracing integration for observability.
#[cfg(feature = "tako-tracing")]
#[cfg_attr(docsrs, doc(cfg(feature = "tako-tracing")))]
pub mod tracing;

/// Core type definitions used throughout the framework.
pub mod types;

/// WebSocket connection handling and message processing.
pub mod ws;

/// GraphQL support (request extractors, responses, and subscriptions).
#[cfg(feature = "async-graphql")]
#[cfg_attr(docsrs, doc(cfg(feature = "async-graphql")))]
pub mod graphql;

/// GraphiQL UI helpers.
#[cfg(feature = "graphiql")]
#[cfg_attr(docsrs, doc(cfg(feature = "graphiql")))]
pub mod graphiql;
pub use bytes::Bytes;
pub use http::{Method, StatusCode, header};
pub use http_body_util::Full;
pub use responder::NOT_FOUND;

/// Starts the HTTP server with the given listener and router.
///
/// This is the main entry point for starting a Tako web server. The function takes
/// ownership of a TCP listener and router, then serves incoming connections until
/// the server is shut down.
///
/// # Examples
///
/// ```rust,no_run
/// use tako::{serve, router::Router};
/// use tokio::net::TcpListener;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let listener = TcpListener::bind("127.0.0.1:8080").await?;
/// let router = Router::new();
/// serve(listener, router).await;
/// # Ok(())
/// # }
/// ```
pub use server::serve;

/// TLS/SSL server implementation for secure connections.
#[cfg(feature = "tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub mod server_tls;

/// Starts the HTTPS server with TLS encryption support.
///
/// Similar to `serve` but enables TLS encryption for secure connections. Requires
/// the "tls" feature to be enabled and proper TLS configuration.
///
/// # Examples
///
/// ```rust,no_run
/// # #[cfg(feature = "tls")]
/// use tako::{serve_tls, router::Router};
/// # #[cfg(feature = "tls")]
/// use tokio::net::TcpListener;
///
/// # #[cfg(feature = "tls")]
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let listener = TcpListener::bind("127.0.0.1:8443").await?;
/// let router = Router::new();
/// // serve_tls(listener, router, tls_config).await;
/// # Ok(())
/// # }
/// ```
#[cfg(feature = "tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use server_tls::serve_tls;

/// Global memory allocator using jemalloc for improved performance.
#[cfg(feature = "jemalloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "jemalloc")))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
