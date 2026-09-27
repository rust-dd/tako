//! Server builders and owned shutdown handles.
//!
//! Use `Server::builder()` for Tokio or `CompioServer::builder()` for Compio.
//! The `try_spawn_*` methods validate configuration and return startup errors;
//! `ServerHandle::result` reports failures after startup. `trigger` stops acceptance,
//! and `shutdown` waits for bounded draining before cancelling remaining work.

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
pub use handle::ServerError;
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
