#![cfg_attr(docsrs, feature(doc_cfg))]

//! Server bootstrap for the Tako framework.
//!
//! Hosts every concrete listener implementation (HTTP/1, TLS, HTTP/3, raw TCP,
//! UDP, Unix sockets, plus the compio variants) and the PROXY protocol parser.
//! Re-exported under the original `tako::*` paths via the umbrella crate.

mod config;
pub use config::AcceptBackoff;
pub use config::H3Congestion;
pub use config::ServerConfig;

#[cfg(not(feature = "compio"))]
mod server;

mod builder;
#[cfg(feature = "tls")]
pub use builder::ClientAuth;
#[cfg(feature = "compio")]
pub use builder::CompioServer;
#[cfg(feature = "compio")]
pub use builder::CompioServerBuilder;
#[cfg(feature = "tls")]
pub use builder::ReloadableResolver;
#[cfg(not(feature = "compio"))]
pub use builder::Server;
#[cfg(not(feature = "compio"))]
pub use builder::ServerBuilder;
pub use builder::ServerHandle;
pub use builder::TlsCert;
#[cfg(feature = "tls")]
pub use builder::build_rustls_server_config;
pub use builder::either;
#[cfg(not(feature = "compio"))]
pub use server::serve;
#[cfg(not(feature = "compio"))]
pub use server::serve_with_config;
#[cfg(not(feature = "compio"))]
pub use server::serve_with_shutdown;
#[cfg(not(feature = "compio"))]
pub use server::serve_with_shutdown_and_config;

#[cfg(feature = "compio")]
#[cfg_attr(docsrs, doc(cfg(feature = "compio")))]
pub mod server_compio;
#[cfg(feature = "compio")]
pub use server_compio::serve;
#[cfg(feature = "compio")]
pub use server_compio::serve_with_config;
#[cfg(feature = "compio")]
pub use server_compio::serve_with_shutdown;
#[cfg(feature = "compio")]
pub use server_compio::serve_with_shutdown_and_config;

/// TLS/SSL server implementation for secure connections.
#[cfg(all(not(feature = "compio-tls"), feature = "tls"))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub mod server_tls;
#[cfg(all(not(feature = "compio"), feature = "tls"))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use server_tls::serve_tls;
#[cfg(all(not(feature = "compio"), feature = "tls"))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use server_tls::serve_tls_with_config;
#[cfg(all(not(feature = "compio"), feature = "tls"))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use server_tls::serve_tls_with_shutdown;
#[cfg(all(not(feature = "compio"), feature = "tls"))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use server_tls::serve_tls_with_shutdown_and_config;

#[cfg(feature = "compio-tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "compio-tls")))]
pub mod server_tls_compio;
#[cfg(feature = "compio-tls")]
pub use server_tls_compio::serve_tls;
#[cfg(feature = "compio-tls")]
pub use server_tls_compio::serve_tls_with_config;
#[cfg(feature = "compio-tls")]
pub use server_tls_compio::serve_tls_with_shutdown;
#[cfg(feature = "compio-tls")]
pub use server_tls_compio::serve_tls_with_shutdown_and_config;

/// HTTP/3 server implementation using QUIC transport.
#[cfg(all(feature = "http3", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub mod server_h3;
#[cfg(all(feature = "http3", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub use server_h3::serve_h3;
#[cfg(all(feature = "http3", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub use server_h3::serve_h3_with_config;
#[cfg(all(feature = "http3", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub use server_h3::serve_h3_with_shutdown;
#[cfg(all(feature = "http3", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub use server_h3::serve_h3_with_shutdown_and_config;

/// Raw TCP server for handling arbitrary TCP connections.
pub mod server_tcp;

/// HTTP/2 cleartext (h2c, prior knowledge) server.
#[cfg(all(feature = "http2", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http2")))]
pub mod server_h2c;
#[cfg(all(feature = "http2", not(feature = "compio")))]
pub use server_h2c::serve_h2c;
#[cfg(all(feature = "http2", not(feature = "compio")))]
pub use server_h2c::serve_h2c_with_config;
#[cfg(all(feature = "http2", not(feature = "compio")))]
pub use server_h2c::serve_h2c_with_shutdown;
#[cfg(all(feature = "http2", not(feature = "compio")))]
pub use server_h2c::serve_h2c_with_shutdown_and_config;

/// UDP datagram server for handling raw UDP packets.
pub mod server_udp;

/// Unix Domain Socket server for local IPC and reverse proxy communication.
#[cfg(all(unix, not(feature = "compio")))]
pub mod server_unix;

/// PROXY protocol v1/v2 parser for load balancer integration.
#[cfg(not(feature = "compio"))]
pub mod proxy_protocol;

/// systemd / s6 / catflap socket activation helpers (LISTEN_FDS).
#[cfg(feature = "socket-activation")]
#[cfg_attr(docsrs, doc(cfg(feature = "socket-activation")))]
pub mod socket_activation;

/// Linux vsock transport for VM-host bridges.
#[cfg(all(target_os = "linux", feature = "vsock", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "vsock")))]
pub mod server_vsock;

mod bind;
pub use bind::bind_with_port_fallback;
