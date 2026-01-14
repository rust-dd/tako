#![cfg(feature = "http3")]
#![cfg_attr(docsrs, doc(cfg(feature = "http3")))]

//! HTTP/3 server implementation using QUIC transport.
//!
//! This module provides HTTP/3 support for Tako web servers using the h3 crate
//! with Quinn as the QUIC transport. HTTP/3 offers improved performance over
//! HTTP/1.1 and HTTP/2 through features like reduced latency, better multiplexing,
//! and built-in encryption via QUIC.
//!
//! # Examples
//!
//! ```rust,no_run
//! # #[cfg(feature = "http3")]
//! use tako::{serve_h3, router::Router, Method, responder::Responder, types::Request};
//!
//! # #[cfg(feature = "http3")]
//! async fn hello(_: Request) -> impl Responder {
//!     "Hello, HTTP/3 World!".into_response()
//! }
//!
//! # #[cfg(feature = "http3")]
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut router = Router::new();
//! router.route(Method::GET, "/", hello);
//! serve_h3(router, "[::]:4433", Some("cert.pem"), Some("key.pem")).await;
//! # Ok(())
//! # }
//! ```

#[cfg(feature = "signals")]
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Buf;
use bytes::Bytes;
use h3::quic::BidiStream;
use h3::server::RequestStream;
use http::Request;
use http_body::Body;
use quinn::crypto::rustls::QuicServerConfig;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::PrivateKeyDer;
use rustls_pemfile::certs;
use rustls_pemfile::pkcs8_private_keys;

use crate::body::TakoBody;
use crate::router::Router;
#[cfg(feature = "signals")]
use crate::signals::Signal;
#[cfg(feature = "signals")]
use crate::signals::SignalArbiter;
#[cfg(feature = "signals")]
use crate::signals::ids;
use crate::types::BoxError;
#[cfg(feature = "signals")]
use crate::types::BuildHasher;

/// Starts an HTTP/3 server with the given router and certificates.
///
/// This function creates a QUIC endpoint and listens for incoming HTTP/3 connections.
/// Unlike TCP-based servers, HTTP/3 uses UDP and QUIC for transport.
///
/// # Arguments
///
/// * `router` - The Tako router containing route definitions
/// * `addr` - The socket address to bind to (e.g., "[::]:4433")
/// * `certs` - Optional path to the TLS certificate file (defaults to "cert.pem")
/// * `key` - Optional path to the TLS private key file (defaults to "key.pem")
pub async fn serve_h3(router: Router, addr: &str, certs: Option<&str>, key: Option<&str>) {
  run(router, addr, certs, key).await.unwrap();
}

/// Runs the HTTP/3 server loop.
async fn run(
  router: Router,
  addr: &str,
  certs: Option<&str>,
  key: Option<&str>,
) -> Result<(), BoxError> {
  #[cfg(feature = "tako-tracing")]
  crate::tracing::init_tracing();

  // Install default crypto provider for rustls (required for QUIC/TLS)
  let _ = rustls::crypto::ring::default_provider().install_default();

  let certs_vec = load_certs(certs.unwrap_or("cert.pem"));
  let key = load_key(key.unwrap_or("key.pem"));

  let mut tls_config = rustls::ServerConfig::builder()
    .with_no_client_auth()
    .with_single_cert(certs_vec, key)?;

  tls_config.max_early_data_size = u32::MAX;
  tls_config.alpn_protocols = vec![b"h3".to_vec()];

  let server_config =
    quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls_config)?));

  let socket_addr: SocketAddr = addr.parse()?;
  let endpoint = quinn::Endpoint::server(server_config, socket_addr)?;

  let router = Arc::new(router);

  #[cfg(feature = "plugins")]
  router.setup_plugins_once();

  let addr_str = endpoint.local_addr()?.to_string();

  #[cfg(feature = "signals")]
  {
    let mut server_meta: HashMap<String, String, BuildHasher> =
      HashMap::with_hasher(BuildHasher::default());
    server_meta.insert("addr".to_string(), addr_str.clone());
    server_meta.insert("transport".to_string(), "quic".to_string());
    server_meta.insert("protocol".to_string(), "h3".to_string());
    SignalArbiter::emit_app(Signal::with_metadata(ids::SERVER_STARTED, server_meta)).await;
  }

  tracing::info!("Tako HTTP/3 listening on {}", addr_str);

  while let Some(new_conn) = endpoint.accept().await {
    let router = router.clone();

    tokio::spawn(async move {
      match new_conn.await {
        Ok(conn) => {
          let remote_addr = conn.remote_address();

          #[cfg(feature = "signals")]
          {
            let mut conn_open_meta: HashMap<String, String, BuildHasher> =
              HashMap::with_hasher(BuildHasher::default());
            conn_open_meta.insert("remote_addr".to_string(), remote_addr.to_string());
            conn_open_meta.insert("protocol".to_string(), "h3".to_string());
            SignalArbiter::emit_app(Signal::with_metadata(
              ids::CONNECTION_OPENED,
              conn_open_meta,
            ))
            .await;
          }

          if let Err(e) = handle_connection(conn, router, remote_addr).await {
            tracing::error!("HTTP/3 connection error: {e}");
          }

          #[cfg(feature = "signals")]
          {
            let mut conn_close_meta: HashMap<String, String, BuildHasher> =
              HashMap::with_hasher(BuildHasher::default());
            conn_close_meta.insert("remote_addr".to_string(), remote_addr.to_string());
            conn_close_meta.insert("protocol".to_string(), "h3".to_string());
            SignalArbiter::emit_app(Signal::with_metadata(
              ids::CONNECTION_CLOSED,
              conn_close_meta,
            ))
            .await;
          }
        }
        Err(e) => {
          tracing::error!("QUIC connection failed: {e}");
        }
      }
    });
  }

  endpoint.wait_idle().await;
  Ok(())
}

/// Handles a single HTTP/3 connection.
async fn handle_connection(
  conn: quinn::Connection,
  router: Arc<Router>,
  remote_addr: SocketAddr,
) -> Result<(), BoxError> {
  let mut h3_conn = h3::server::Connection::new(h3_quinn::Connection::new(conn)).await?;

  loop {
    match h3_conn.accept().await {
      Ok(Some(resolver)) => {
        let router = router.clone();
        tokio::spawn(async move {
          match resolver.resolve_request().await {
            Ok((req, stream)) => {
              if let Err(e) = handle_request(req, stream, router, remote_addr).await {
                tracing::error!("HTTP/3 request error: {e}");
              }
            }
            Err(e) => {
              tracing::error!("HTTP/3 request resolve error: {e}");
            }
          }
        });
      }
      Ok(None) => {
        break;
      }
      Err(e) => {
        tracing::error!("HTTP/3 accept error: {e}");
        break;
      }
    }
  }

  Ok(())
}

/// Handles a single HTTP/3 request.
async fn handle_request<S>(
  req: Request<()>,
  mut stream: RequestStream<S, Bytes>,
  router: Arc<Router>,
  remote_addr: SocketAddr,
) -> Result<(), BoxError>
where
  S: BidiStream<Bytes>,
{
  #[cfg(feature = "signals")]
  let path = req.uri().path().to_string();
  #[cfg(feature = "signals")]
  let method = req.method().to_string();

  #[cfg(feature = "signals")]
  {
    let mut req_meta: HashMap<String, String, BuildHasher> =
      HashMap::with_hasher(BuildHasher::default());
    req_meta.insert("method".to_string(), method.clone());
    req_meta.insert("path".to_string(), path.clone());
    req_meta.insert("protocol".to_string(), "h3".to_string());
    SignalArbiter::emit_app(Signal::with_metadata(ids::REQUEST_STARTED, req_meta)).await;
  }

  // Collect request body
  let mut body_bytes = Vec::new();
  while let Some(mut chunk) = stream.recv_data().await? {
    while chunk.has_remaining() {
      let bytes = chunk.chunk();
      body_bytes.extend_from_slice(bytes);
      chunk.advance(bytes.len());
    }
  }

  // Build request with body
  let (parts, _) = req.into_parts();
  let body = TakoBody::from(Bytes::from(body_bytes));
  let mut tako_req = Request::from_parts(parts, body);
  tako_req.extensions_mut().insert(remote_addr);

  // Dispatch through router
  let response = router.dispatch(tako_req).await;

  #[cfg(feature = "signals")]
  {
    let mut done_meta: HashMap<String, String, BuildHasher> =
      HashMap::with_hasher(BuildHasher::default());
    done_meta.insert("method".to_string(), method);
    done_meta.insert("path".to_string(), path);
    done_meta.insert("status".to_string(), response.status().as_u16().to_string());
    done_meta.insert("protocol".to_string(), "h3".to_string());
    SignalArbiter::emit_app(Signal::with_metadata(ids::REQUEST_COMPLETED, done_meta)).await;
  }

  // Send response
  let (parts, body) = response.into_parts();
  let resp = http::Response::from_parts(parts, ());

  stream.send_response(resp).await?;

  // Stream body data frame by frame (supports SSE)
  let mut body = std::pin::pin!(body);
  while let Some(frame) = std::future::poll_fn(|cx| body.as_mut().poll_frame(cx)).await {
    match frame {
      Ok(frame) => {
        if let Some(data) = frame.data_ref().filter(|d| !d.is_empty()) {
          stream.send_data(data.clone()).await?;
        }
      }
      Err(e) => {
        tracing::error!("HTTP/3 body frame error: {e}");
        break;
      }
    }
  }

  stream.finish().await?;

  Ok(())
}

/// Loads TLS certificates from a PEM-encoded file.
pub fn load_certs(path: &str) -> Vec<CertificateDer<'static>> {
  let mut rd = BufReader::new(File::open(path).unwrap());
  certs(&mut rd).map(|r| r.expect("bad cert")).collect()
}

/// Loads a private key from a PEM-encoded file.
pub fn load_key(path: &str) -> PrivateKeyDer<'static> {
  let mut rd = BufReader::new(File::open(path).unwrap());
  pkcs8_private_keys(&mut rd)
    .next()
    .expect("no private key found")
    .expect("bad private key")
    .into()
}
