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

use std::io::ErrorKind;
use std::io::Write;
use std::io::{self};
use std::net::SocketAddr;
use std::str::FromStr;

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
#[cfg(not(feature = "compio"))]
mod server;

/// Server-Sent Events (SSE) support for real-time communication.
pub mod sse;

/// Application state management and dependency injection.
pub mod state;

#[cfg(feature = "signals")]
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
#[cfg(not(feature = "compio"))]
pub mod ws;

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

#[cfg(feature = "zero-copy-extractors")]
#[cfg_attr(docsrs, doc(cfg(feature = "zero-copy-extractors")))]
pub mod zero_copy_extractors;

pub use bytes::Bytes;
pub use http::Method;
pub use http::StatusCode;
pub use http::header;
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
#[cfg(not(feature = "compio"))]
pub use server::serve;
#[cfg(feature = "compio")]
pub use server_compio::serve;

/// Bind a TCP listener for `addr`, asking interactively to increment the port
/// if it is already in use.
///
/// This helper is primarily intended for local development and example binaries.
/// It will keep proposing the next port number until a free one is found or
/// the user declines.
///
/// # Errors
///
/// Returns an error if:
/// - The provided address string is not a valid socket address.
/// - The port is already in use and the user declines to try another port.
/// - A non-recoverable I/O error occurs during binding.
#[cfg(not(feature = "compio"))]
pub async fn bind_with_port_fallback(addr: &str) -> io::Result<tokio::net::TcpListener> {
  let mut socket_addr =
    SocketAddr::from_str(addr).map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?;
  let start_port = socket_addr.port();

  loop {
    let addr_str = socket_addr.to_string();
    match tokio::net::TcpListener::bind(&addr_str).await {
      Ok(listener) => {
        if socket_addr.port() != start_port {
          println!(
            "Port {} was in use, starting on {} instead",
            start_port,
            socket_addr.port()
          );
        }
        return Ok(listener);
      }
      Err(err) if err.kind() == ErrorKind::AddrInUse => {
        let next_port = socket_addr.port().saturating_add(1);
        if !ask_to_use_next_port(socket_addr.port(), next_port)? {
          return Err(err);
        }
        socket_addr.set_port(next_port);
      }
      Err(err) => return Err(err),
    }
  }
}

/// Bind a TCP listener for `addr`, asking interactively to increment the port
/// if it is already in use (compio version).
///
/// # Errors
///
/// Returns an error if:
/// - The provided address string is not a valid socket address.
/// - The port is already in use and the user declines to try another port.
/// - A non-recoverable I/O error occurs during binding.
#[cfg(feature = "compio")]
pub async fn bind_with_port_fallback(addr: &str) -> io::Result<compio::net::TcpListener> {
  let mut socket_addr =
    SocketAddr::from_str(addr).map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?;
  let start_port = socket_addr.port();

  loop {
    let addr_str = socket_addr.to_string();
    match compio::net::TcpListener::bind(&addr_str).await {
      Ok(listener) => {
        if socket_addr.port() != start_port {
          println!(
            "Port {} was in use, starting on {} instead",
            start_port,
            socket_addr.port()
          );
        }
        return Ok(listener);
      }
      Err(err) if err.kind() == ErrorKind::AddrInUse => {
        let next_port = socket_addr.port().saturating_add(1);
        if !ask_to_use_next_port(socket_addr.port(), next_port)? {
          return Err(err);
        }
        socket_addr.set_port(next_port);
      }
      Err(err) => return Err(err),
    }
  }
}

fn ask_to_use_next_port(current: u16, next: u16) -> io::Result<bool> {
  loop {
    print!(
      "Port {} is already in use. Start on {} instead? [Y/n]: ",
      current, next
    );
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();

    if trimmed.is_empty()
      || trimmed.eq_ignore_ascii_case("y")
      || trimmed.eq_ignore_ascii_case("yes")
    {
      return Ok(true);
    }

    if trimmed.eq_ignore_ascii_case("n") || trimmed.eq_ignore_ascii_case("no") {
      return Ok(false);
    }

    println!("Please answer 'y' or 'n'.");
  }
}

/// TLS/SSL server implementation for secure connections.
#[cfg(all(not(feature = "compio-tls"), feature = "tls"))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub mod server_tls;

/// Compio server implementation for efficient I/O operations.
#[cfg(feature = "compio")]
#[cfg_attr(docsrs, doc(cfg(feature = "compio")))]
pub mod server_compio;

#[cfg(feature = "compio-tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "compio")))]
pub mod server_tls_compio;

/// HTTP/3 server implementation using QUIC transport.
#[cfg(feature = "http3")]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub mod server_h3;

/// Starts the HTTP/3 server with QUIC transport.
///
/// HTTP/3 provides improved performance through QUIC's features like reduced
/// latency, better multiplexing, and built-in encryption.
///
/// # Examples
///
/// ```rust,no_run
/// # #[cfg(feature = "http3")]
/// use tako::{serve_h3, router::Router};
///
/// # #[cfg(feature = "http3")]
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let router = Router::new();
/// serve_h3(router, "[::]:4433", Some("cert.pem"), Some("key.pem")).await;
/// # Ok(())
/// # }
/// ```
#[cfg(feature = "http3")]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub use server_h3::serve_h3;
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
#[cfg(all(not(feature = "compio"), feature = "tls"))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use server_tls::serve_tls;
#[cfg(feature = "compio-tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use server_tls_compio::serve_tls;

/// Global memory allocator using jemalloc for improved performance.
#[cfg(feature = "jemalloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "jemalloc")))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
