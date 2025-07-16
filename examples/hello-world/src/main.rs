#![allow(unused)]
//! # Hello World Example
//!
//! This example demonstrates a minimal Tako web server that responds with "Hello, World!" to
//! HTTP GET requests at the root path (`/`). It showcases how to define a simple request handler,
//! set up a router, and start the server.
//!
//! Tako is a lightweight and modular web framework for Rust, designed for building asynchronous
//! web applications. This example is a great starting point for understanding the basics of
//! routing and request handling in Tako.
//!
//! # Examples
//!
//! To run this example, execute the following command:
//!
//! ```bash
//! cargo run --example hello-world
//! ```
//!
//! Then, open your browser or use `curl` to access the server:
//!
//! ```bash
//! curl http://127.0.0.1:8080
//! ```
//!
//! You should see the response:
//!
//! ```text
//! Hello, World!
//! ```

use anyhow::Result;
use tako::{Method, responder::Responder, router::Router, types::Request};
use tokio::net::TcpListener;

/// Handles HTTP GET requests to the root path (`/`).
///
/// This function responds with a plain text message: "Hello, World!".
///
/// # Examples
///
/// ```rust
/// use tako::responder::Responder;
/// use tako::types::Request;
///
/// async fn hello_world(_: Request) -> impl Responder {
///     "Hello, World!".into_response()
/// }
/// ```
async fn hello_world(_: Request) -> impl Responder {
    "Hello, World!".into_response()
}

#[tokio::main]
/// Entry point for the Hello World example server.
///
/// This function sets up a TCP listener, initializes a router with a single route,
/// and starts the Tako server. The server listens on `127.0.0.1:8080` and responds
/// to HTTP GET requests at the root path (`/`) with "Hello, World!".
///
/// # Errors
///
/// Returns an error if the TCP listener fails to bind to the specified address or
/// if the server encounters an issue during runtime.
///
/// # Examples
///
/// Run the server:
///
/// ```bash
/// cargo run --example hello-world
/// ```
///
/// Access the server:
///
/// ```bash
/// curl http://127.0.0.1:8080
/// ```
///
/// Expected response:
///
/// ```text
/// Hello, World!
/// ```
async fn main() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:8080").await?;

    let mut router = Router::new();
    router.route(Method::GET, "/", hello_world);

    println!("Server running at http://127.0.0.1:8080");
    tako::serve(listener, router).await;

    Ok(())
}
