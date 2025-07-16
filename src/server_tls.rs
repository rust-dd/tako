#![cfg(feature = "tls")]

//! TLS-enabled HTTP server implementation for secure connections.
//!
//! This module provides TLS/SSL support for Tako web servers using rustls for encryption.
//! It handles secure connection establishment, certificate loading, and supports both
//! HTTP/1.1 and HTTP/2 protocols (when the http2 feature is enabled). The main entry
//! point is `serve_tls` which starts a secure server with the provided certificates.
//!
//! # Examples
//!
//! ```rust,no_run
//! # #[cfg(feature = "tls")]
//! use tako::{serve_tls, router::Router, Method, responder::Responder, types::Request};
//! # #[cfg(feature = "tls")]
//! use tokio::net::TcpListener;
//!
//! # #[cfg(feature = "tls")]
//! async fn hello(_: Request) -> impl Responder {
//!     "Hello, Secure World!".into_response()
//! }
//!
//! # #[cfg(feature = "tls")]
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let listener = TcpListener::bind("127.0.0.1:8443").await?;
//! let mut router = Router::new();
//! router.route(Method::GET, "/", hello);
//! serve_tls(listener, router, Some("cert.pem"), Some("key.pem")).await;
//! # Ok(())
//! # }
//! ```

use hyper::{
    Request,
    server::conn::{http1, http2},
    service::service_fn,
};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls_pemfile::{certs, pkcs8_private_keys};
use std::{convert::Infallible, fs::File, io::BufReader, sync::Arc};
use tokio::net::TcpListener;
use tokio_rustls::{TlsAcceptor, rustls::ServerConfig};

use crate::{router::Router, types::BoxError};

/// Starts a TLS-enabled HTTP server with the given listener, router, and certificates.
pub async fn serve_tls(
    listener: TcpListener,
    router: Router,
    certs: Option<&str>,
    key: Option<&str>,
) {
    run(listener, router, certs, key).await.unwrap();
}

/// Runs the TLS server loop, handling secure connections and request dispatch.
pub async fn run(
    listener: TcpListener,
    router: Router,
    certs: Option<&str>,
    key: Option<&str>,
) -> Result<(), BoxError> {
    #[cfg(feature = "tako-tracing")]
    crate::tracing::init_tracing();

    let certs = load_certs(certs.unwrap_or("cert.pem"));
    let key = load_key(key.unwrap_or("key.pem"));

    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();

    #[cfg(feature = "http2")]
    {
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    }

    #[cfg(not(feature = "http2"))]
    {
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
    }

    let acceptor = TlsAcceptor::from(Arc::new(config));
    let router = Arc::new(router);

    // Setup plugins
    #[cfg(feature = "plugins")]
    router.setup_plugins_once();

    println!("Tako TLS listening on {}", listener.local_addr()?);

    loop {
        let (stream, addr) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let router = router.clone();

        tokio::spawn(async move {
            let tls_stream = match acceptor.accept(stream).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("TLS error: {e}");
                    return;
                }
            };

            #[cfg(feature = "http2")]
            let proto = tls_stream.get_ref().1.alpn_protocol().map(|p| p.to_vec());

            let io = TokioIo::new(tls_stream);
            let svc = service_fn(move |mut req: Request<_>| {
                let r = router.clone();
                async move {
                    req.extensions_mut().insert(addr);
                    Ok::<_, Infallible>(r.dispatch(req).await)
                }
            });

            #[cfg(feature = "http2")]
            if proto.as_deref() == Some(b"h2") {
                let h2 = http2::Builder::new(TokioExecutor::new());

                if let Err(e) = h2.serve_connection(io, svc).await {
                    eprintln!("HTTP/2 error: {e}");
                }
                return;
            }

            let mut h1 = http1::Builder::new();
            h1.keep_alive(true);

            if let Err(e) = h1.serve_connection(io, svc).with_upgrades().await {
                eprintln!("HTTP/1.1 error: {e}");
            }
        });
    }
}

/// Loads TLS certificates from a PEM-encoded file.
///
/// Reads and parses X.509 certificates from the specified file path. The file
/// should contain one or more PEM-encoded certificates.
///
/// # Arguments
///
/// * `path` - File system path to the certificate file
///
/// # Panics
///
/// Panics if the file cannot be opened, read, or if the certificates are
/// malformed or invalid.
///
/// # Examples
///
/// ```rust,no_run
/// # #[cfg(feature = "tls")]
/// use tako::server_tls::load_certs;
///
/// # #[cfg(feature = "tls")]
/// # fn example() {
/// let certs = load_certs("server.crt");
/// println!("Loaded {} certificates", certs.len());
/// # }
/// ```
fn load_certs(path: &str) -> Vec<CertificateDer<'static>> {
    let mut rd = BufReader::new(File::open(path).unwrap());
    certs(&mut rd).map(|r| r.expect("bad cert")).collect()
}

/// Loads a private key from a PEM-encoded file.
///
/// Reads and parses a PKCS#8 private key from the specified file path. The file
/// should contain a single PEM-encoded private key.
///
/// # Arguments
///
/// * `path` - File system path to the private key file
///
/// # Panics
///
/// Panics if the file cannot be opened, read, if no private key is found,
/// or if the private key is malformed or invalid.
///
/// # Examples
///
/// ```rust,no_run
/// # #[cfg(feature = "tls")]
/// use tako::server_tls::load_key;
///
/// # #[cfg(feature = "tls")]
/// # fn example() {
/// let key = load_key("server.key");
/// println!("Loaded private key successfully");
/// # }
/// ```
fn load_key(path: &str) -> PrivateKeyDer<'static> {
    let mut rd = BufReader::new(File::open(path).unwrap());
    pkcs8_private_keys(&mut rd)
        .next()
        .expect("no private key found")
        .expect("bad private key")
        .into()
}
