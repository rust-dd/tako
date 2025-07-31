//! A lightweight and modular web framework for building async applications in Rust.
//!
//! Tako provides core components for routing, middleware, request handling, and response
//! generation. The framework is designed around composable modules that can be mixed and
//! matched based on application needs. Key types include `Router` for routing requests,
//! various extractors for parsing request data, and responders for generating responses.
//!
//! # Examples
//!
//! ```rust
//! use tako::{Method, router::Router, responder::Responder, types::Request};
//!
//! async fn hello(_: Request) -> impl Responder {
//!     "Hello, World!".into_response()
//! }
//!
//! let mut router = Router::new();
//! router.route(Method::GET, "/", hello);
//! ```

/// HTTP request and response body handling utilities.
pub mod body;

/// HTTP client implementation for making outbound requests.
#[cfg(feature = "client")]
pub mod client;

/// Request data extraction utilities for parsing query params, JSON, and more.
pub mod extractors;

/// File streaming utilities for serving files.
#[cfg(feature = "file-stream")]
pub mod file_stream;

/// Request handler traits and implementations.
mod handler;

/// Middleware for processing requests and responses in a pipeline.
pub mod middleware;

/// Plugin system for extending framework functionality.
#[cfg(feature = "plugins")]
pub mod plugins;

/// Response generation utilities and traits.
pub mod responder;

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

/// Static file serving utilities.
pub mod r#static;

/// Distributed tracing integration for observability.
#[cfg(feature = "tako-tracing")]
pub mod tracing;

/// Core type definitions used throughout the framework.
pub mod types;

/// WebSocket connection handling and message processing.
pub mod ws;

pub use bytes::Bytes;
pub use hyper::{Method, StatusCode};

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
pub use server_tls::serve_tls;

/// Global memory allocator using jemalloc for improved performance.
#[cfg(feature = "jemalloc")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
