//! Tako: A lightweight web framework for building asynchronous web applications in Rust.
//!
//! This library provides a modular and extensible framework for creating web servers,
//! handling requests, and managing application state. It is designed to be fast, ergonomic,
//! and easy to use.

/// Module for handling HTTP request and response bodies.
pub mod body;

/// Module for working with byte streams and buffers.
mod bytes;

/// Module for extracting data from requests, such as query parameters or JSON payloads.
pub mod extractors;

/// Module for defining and managing request handlers.
mod handler;

/// Module for defining and managing middleware.
pub mod middleware;

/// Module for defining and managing plugins.
#[cfg(feature = "plugins")]
pub mod plugins;

/// Module for creating and sending HTTP responses.
pub mod responder;

/// Module for defining application routes and their handlers.
mod route;

/// Module for managing the application's routing logic.
pub mod router;

/// Module for starting and managing the web server.
mod server;

/// Module for handling Server-Sent Events (SSE).
pub mod sse;

/// Module for managing application state and shared data.
pub mod state;

/// Module for defining and working with custom types used in the framework.
pub mod types;

/// Module for handling WebSocket connections.
pub mod ws;

pub use hyper::Method;
pub use server::serve;

/// Module for enabling TLS support in the server.
#[cfg(feature = "tls")]
pub mod server_tls;

#[cfg(feature = "tls")]
pub use server_tls::serve_tls;
