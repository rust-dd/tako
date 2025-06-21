#![cfg(feature = "tls")]

/// This module provides functionality for serving Tako applications over TLS.
/// It includes methods for setting up a TLS server and handling secure connections.
use hyper::{Request, server::conn::http1, service::service_fn};
use hyper_util::rt::TokioIo;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls_pemfile::{certs, pkcs8_private_keys};
use std::{convert::Infallible, fs::File, io::BufReader, sync::Arc};
use tokio::net::TcpListener;
use tokio_rustls::{TlsAcceptor, rustls::ServerConfig};

use crate::{router::Router, types::BoxedError};

/// Starts a TLS server using the provided `TcpListener` and `Router`.
///
/// # Arguments
///
/// * `listener` - A `TcpListener` that listens for incoming connections.
/// * `router` - The `Router` instance that handles routing and request dispatch.
///
/// # Panics
///
/// This function will panic if the server encounters an unrecoverable error.
pub async fn serve_tls(listener: TcpListener, router: Router) {
    run(listener, router).await.unwrap();
}

/// Runs the TLS server, accepting connections and dispatching requests.
///
/// # Arguments
///
/// * `listener` - A `TcpListener` that listens for incoming connections.
/// * `router` - The `Router` instance that handles routing and request dispatch.
///
/// # Returns
///
/// A `Result` indicating success or failure.
///
/// # Errors
///
/// Returns an error if the server fails to accept connections or handle requests.
pub async fn run(listener: TcpListener, router: Router) -> Result<(), BoxedError> {
    let certs = load_certs("cert.pem");
    let key = load_key("key.pem");

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();

    let acceptor = TlsAcceptor::from(Arc::new(config));
    let router = Arc::new(router);

    println!("Tako TLS listening on {}", listener.local_addr()?);

    loop {
        let (stream, addr) = listener.accept().await?;
        println!("Accepted TLS connection from {addr}");
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

            let io = TokioIo::new(tls_stream);
            let svc = service_fn(move |req: Request<_>| {
                let r = router.clone();
                async move { Ok::<_, Infallible>(r.dispatch(req).await) }
            });

            let mut http = http1::Builder::new();
            http.keep_alive(true);

            if let Err(e) = http.serve_connection(io, svc).with_upgrades().await {
                eprintln!("Connection error: {e}");
            }
        });
    }
}

/// Loads TLS certificates from the specified file path.
///
/// # Arguments
///
/// * `path` - The file path to the certificate file.
///
/// # Returns
///
/// A vector of `CertificateDer` objects representing the loaded certificates.
///
/// # Panics
///
/// This function will panic if the file cannot be opened or the certificates are invalid.
fn load_certs(path: &str) -> Vec<CertificateDer<'static>> {
    let mut rd = BufReader::new(File::open(path).unwrap());
    certs(&mut rd).map(|r| r.expect("bad cert")).collect()
}

/// Loads a private key from the specified file path.
///
/// # Arguments
///
/// * `path` - The file path to the private key file.
///
/// # Returns
///
/// A `PrivateKeyDer` object representing the loaded private key.
///
/// # Panics
///
/// This function will panic if the file cannot be opened or the private key is invalid.
fn load_key(path: &str) -> PrivateKeyDer<'static> {
    let mut rd = BufReader::new(File::open(path).unwrap());
    pkcs8_private_keys(&mut rd)
        .next()
        .expect("no private key found")
        .expect("bad private key")
        .into()
}
