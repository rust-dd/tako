//! Unix Domain Socket server for local IPC and reverse proxy communication.
//!
//! Provides both raw Unix socket and HTTP-over-Unix-socket servers.
//! The HTTP variant is ideal for production deployments behind nginx/`HAProxy`
//! where the app communicates via a local socket file instead of TCP.
//!
//! Filesystem and Linux abstract-namespace paths are both supported. A path
//! whose string representation starts with `@` is interpreted as a Linux
//! abstract socket: e.g. `@tako.sock` binds to the abstract name `tako.sock`
//! (NUL-prefixed in the kernel). Abstract sockets do not touch the filesystem,
//! so the stale-socket cleanup and post-shutdown removal are skipped for them.
//!
//! # Examples
//!
//! ## Raw Unix socket (echo server)
//! ```rust,no_run
//! use tako::server_unix::serve_unix;
//! use tokio::io::{AsyncReadExt, AsyncWriteExt};
//!
//! # async fn example() -> std::io::Result<()> {
//! serve_unix("/tmp/tako.sock", |mut stream, _addr| {
//!     Box::pin(async move {
//!         let mut buf = vec![0u8; 4096];
//!         let n = stream.read(&mut buf).await?;
//!         stream.write_all(&buf[..n]).await?;
//!         Ok(())
//!     })
//! }).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## HTTP over Unix socket
//! ```rust,no_run
//! use tako::server_unix::serve_unix_http;
//! use tako::router::Router;
//!
//! # async fn example() -> std::io::Result<()> {
//! let router = Router::new();
//! serve_unix_http("/tmp/tako-http.sock", router).await;
//! # Ok(())
//! # }
//! ```

mod http;
mod listener;
mod raw;

pub use http::serve_unix_http;
pub use http::serve_unix_http_with_config;
pub use http::serve_unix_http_with_shutdown;
pub use http::serve_unix_http_with_shutdown_and_config;
pub use listener::UnixPeerAddr;
pub use raw::serve_unix;
pub use raw::serve_unix_with_shutdown;
pub use raw::serve_unix_with_shutdown_and_drain;
