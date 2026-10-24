#![cfg_attr(docsrs, feature(doc_cfg))]

//! Server builders and listeners for HTTP/1, h2c, TLS, HTTP/3, raw TCP/UDP, Unix sockets and the
//! PROXY protocol; use them through the `tako` umbrella crate.

pub use tako_rs_core::server_support::shutdown_signal;

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
pub use builder::ServerError;
pub use builder::ServerHandle;
pub use builder::TlsCert;
#[cfg(feature = "tls")]
pub use builder::build_rustls_server_config;

#[cfg(feature = "http2")]
mod h2_settings;

#[cfg(all(feature = "compio", feature = "http2"))]
mod compio_h2;

#[cfg(feature = "compio")]
mod server_compio;

#[cfg(all(not(feature = "compio"), feature = "tls"))]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub mod server_tls;

#[cfg(feature = "compio-tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "compio-tls")))]
pub mod server_tls_compio;

#[cfg(feature = "http3")]
mod h3_common;

#[cfg(feature = "webtransport")]
#[cfg_attr(docsrs, doc(cfg(feature = "webtransport")))]
pub mod webtransport;

#[cfg(all(feature = "http3", not(feature = "compio")))]
#[cfg_attr(docsrs, doc(cfg(feature = "http3")))]
pub mod server_h3;

pub mod server_tcp;

#[cfg(all(feature = "http2", not(feature = "compio")))]
mod server_h2c;

#[cfg(feature = "udp")]
#[cfg_attr(docsrs, doc(cfg(feature = "udp")))]
pub mod server_udp;

#[cfg(unix)]
pub mod server_unix;

#[cfg(feature = "proxy-protocol")]
#[cfg_attr(docsrs, doc(cfg(feature = "proxy-protocol")))]
pub mod proxy_protocol;

#[cfg(feature = "socket-activation")]
#[cfg_attr(docsrs, doc(cfg(feature = "socket-activation")))]
pub mod socket_activation;

#[cfg(all(target_os = "linux", feature = "vsock", not(feature = "compio")))]
mod server_vsock;

mod bind;
pub use bind::bind_with_port_fallback;

#[cfg(unix)]
mod unix_path;
