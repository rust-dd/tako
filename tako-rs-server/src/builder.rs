//! `Server` (Tokio) and `CompioServer` builders whose `try_spawn_*` methods return startup errors,
//! and the `ServerHandle` that stops and drains them.

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
