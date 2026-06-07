//! Unified [`Server`] / [`CompioServer`] builder fronting every Tako transport.
//!
//! The direct `serve_*` / `serve_*_with_shutdown` / `*_with_config` functions
//! still exist and keep working. This module is an additive convenience layer:
//! pick a transport via `spawn_*`, hand it a [`crate::ServerConfig`], and get
//! back a [`ServerHandle`] that owns a shutdown trigger.
//!
//! The handle itself is runtime-agnostic — both [`Server`] (tokio) and
//! [`CompioServer`] (cfg `compio`) return the same [`ServerHandle`] type.
//! Internally each `spawn_*` wraps the underlying `serve_*` future so that
//! when it returns, a `done` [`Notify`] is signalled. [`ServerHandle::join`]
//! awaits that notify; [`ServerHandle::shutdown`] triggers the shutdown
//! signal and then awaits the same `done`.
//!
//! No additional allocation or atomic swap is introduced on the per-connection
//! / per-request hot path — the spawn wrapper is a single async block over the
//! underlying `serve_*_with_shutdown_and_config` call.

mod handle;
mod spawn;
mod tls_cert;

#[cfg(feature = "compio")]
mod compio_server;
#[cfg(not(feature = "compio"))]
mod tokio_server;

#[cfg(feature = "compio")]
pub use compio_server::CompioServer;
#[cfg(feature = "compio")]
pub use compio_server::CompioServerBuilder;
pub use handle::ServerHandle;
pub use handle::either;
#[cfg(feature = "tls")]
pub use tls_cert::ClientAuth;
#[cfg(feature = "tls")]
pub use tls_cert::ReloadableResolver;
pub use tls_cert::TlsCert;
#[cfg(feature = "tls")]
pub use tls_cert::build_rustls_server_config;
#[cfg(not(feature = "compio"))]
pub use tokio_server::Server;
#[cfg(not(feature = "compio"))]
pub use tokio_server::ServerBuilder;
