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
//! # #[cfg(not(feature = "compio"))]
//! # async fn example() -> std::io::Result<()> {
//! use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
//! # #[cfg(feature = "compio")]
//! # async fn example() -> std::io::Result<()> {
//! # use compio::io::{AsyncRead, AsyncWriteExt};
//! # serve_unix("/tmp/tako.sock", |mut stream, _peer| Box::pin(async move {
//! #     let compio::BufResult(result, mut buf) = stream.read(vec![0u8; 4096]).await;
//! #     buf.truncate(result?);
//! #     let compio::BufResult(result, _) = stream.write_all(buf).await;
//! #     result
//! # })).await
//! # }
//! ```
//!
//! ## HTTP over Unix socket
//! ```rust,no_run
//! # #[cfg(not(feature = "compio"))]
//! # async fn example() -> Result<(), tako::types::BoxError> {
//! use tako::router::Router;
//! use tako::Server;
//!
//! let router = Router::new();
//! let handle = Server::builder()
//!     .build()
//!     .try_spawn_unix_http("/tmp/tako-http.sock", router)
//!     .await?;
//! handle.result().await?;
//! # Ok(())
//! # }
//! # #[cfg(feature = "compio")]
//! # async fn example() -> Result<(), tako::types::BoxError> {
//! # let router = tako::router::Router::new();
//! # let handle = tako::CompioServer::builder()
//! #     .build()
//! #     .try_spawn_unix_http("/tmp/tako-http.sock", router)
//! #     .await?;
//! # handle.result().await?;
//! # Ok(())
//! # }
//! ```

#[cfg(not(feature = "compio"))]
pub(crate) mod http;
#[cfg(not(feature = "compio"))]
pub(crate) mod listener;
#[cfg(not(feature = "compio"))]
mod raw;
#[cfg(feature = "compio")]
mod raw_compio;

#[cfg(not(feature = "compio"))]
#[allow(deprecated)]
pub use http::serve_unix_http;
#[cfg(not(feature = "compio"))]
#[allow(deprecated)]
pub use http::serve_unix_http_with_config;
#[cfg(not(feature = "compio"))]
#[allow(deprecated)]
pub use http::serve_unix_http_with_shutdown;
#[cfg(not(feature = "compio"))]
#[allow(deprecated)]
pub use http::serve_unix_http_with_shutdown_and_config;
#[cfg(not(feature = "compio"))]
pub use raw::serve_unix;
#[cfg(not(feature = "compio"))]
pub use raw::serve_unix_with_shutdown;
#[cfg(not(feature = "compio"))]
pub use raw::serve_unix_with_shutdown_and_drain;
#[cfg(feature = "compio")]
pub use raw_compio::serve_unix;
#[cfg(feature = "compio")]
pub use raw_compio::serve_unix_with_shutdown;
#[cfg(feature = "compio")]
pub use raw_compio::serve_unix_with_shutdown_and_drain;
pub use tako_rs_core::conn_info::UnixPeerAddr;
