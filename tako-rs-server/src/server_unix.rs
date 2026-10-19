//! Raw and HTTP servers on Unix domain sockets. A path starting with `@` names a Linux abstract
//! socket, which skips the stale-file cleanup and the removal after shutdown.
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
