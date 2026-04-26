#![cfg_attr(docsrs, doc(cfg(feature = "webtransport")))]

//! WebTransport server support over QUIC.
//!
//! Provides WebTransport sessions with bidirectional/unidirectional streams
//! and unreliable datagram support. Built on top of Quinn (QUIC) with
//! the existing HTTP/3 infrastructure.
//!
//! # Examples
//!
//! ```rust,no_run
//! # #[cfg(feature = "webtransport")]
//! use tako::webtransport::{serve_webtransport, WebTransportSession};
//!
//! # #[cfg(feature = "webtransport")]
//! # async fn example() {
//! serve_webtransport("[::]:4433", "cert.pem", "key.pem", |session| {
//!     Box::pin(async move {
//!         // Accept bidirectional streams
//!         while let Ok((mut send, mut recv)) = session.accept_bi().await {
//!             tokio::spawn(async move {
//!                 let mut buf = vec![0u8; 4096];
//!                 while let Ok(Some(n)) = recv.read(&mut buf).await {
//!                     let _ = send.write_all(&buf[..n]).await;
//!                 }
//!             });
//!         }
//!     })
//! }).await;
//! # }
//! ```

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use quinn::crypto::rustls::QuicServerConfig;

/// Default drain timeout for graceful shutdown.
const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

/// A WebTransport session wrapping a QUIC connection.
///
/// Provides access to bidirectional/unidirectional streams and unreliable
/// datagrams over a QUIC connection.
pub struct WebTransportSession {
  conn: quinn::Connection,
}

impl WebTransportSession {
  /// Creates a new session from a QUIC connection.
  pub fn new(conn: quinn::Connection) -> Self {
    Self { conn }
  }

  /// Returns the remote address of the peer.
  pub fn remote_address(&self) -> SocketAddr {
    self.conn.remote_address()
  }

  /// Accepts an incoming bidirectional stream.
  pub async fn accept_bi(
    &self,
  ) -> Result<(quinn::SendStream, quinn::RecvStream), quinn::ConnectionError> {
    self.conn.accept_bi().await
  }

  /// Accepts an incoming unidirectional stream.
  pub async fn accept_uni(&self) -> Result<quinn::RecvStream, quinn::ConnectionError> {
    self.conn.accept_uni().await
  }

  /// Opens a new bidirectional stream.
  pub async fn open_bi(
    &self,
  ) -> Result<(quinn::SendStream, quinn::RecvStream), quinn::ConnectionError> {
    self.conn.open_bi().await
  }

  /// Opens a new unidirectional stream.
  pub async fn open_uni(&self) -> Result<quinn::SendStream, quinn::ConnectionError> {
    self.conn.open_uni().await
  }

  /// Reads an unreliable datagram from the peer.
  pub async fn read_datagram(&self) -> Result<bytes::Bytes, quinn::ConnectionError> {
    self.conn.read_datagram().await
  }

  /// Sends an unreliable datagram to the peer.
  pub fn send_datagram(
    &self,
    data: bytes::Bytes,
  ) -> Result<(), quinn::SendDatagramError> {
    self.conn.send_datagram(data)
  }

  /// Closes the session with a reason.
  pub fn close(&self, code: u32, reason: &[u8]) {
    self.conn.close(quinn::VarInt::from_u32(code), reason);
  }

  /// Returns a reference to the underlying QUIC connection.
  pub fn connection(&self) -> &quinn::Connection {
    &self.conn
  }
}

/// Handler function type for WebTransport sessions.
pub type WebTransportHandler = Arc<
  dyn Fn(WebTransportSession) -> Pin<Box<dyn Future<Output = ()> + Send>>
    + Send
    + Sync,
>;

/// Starts a WebTransport server on the given address.
///
/// Each accepted QUIC connection is wrapped in a `WebTransportSession` and
/// dispatched to the handler.
pub async fn serve_webtransport<F>(
  addr: &str,
  cert_path: &str,
  key_path: &str,
  handler: F,
) where
  F: Fn(WebTransportSession) -> Pin<Box<dyn Future<Output = ()> + Send>>
    + Send
    + Sync
    + 'static,
{
  if let Err(e) = run(addr, cert_path, key_path, handler, None::<std::future::Pending<()>>).await {
    tracing::error!("WebTransport server error: {e}");
  }
}

/// Starts a WebTransport server with graceful shutdown support.
pub async fn serve_webtransport_with_shutdown<F, S>(
  addr: &str,
  cert_path: &str,
  key_path: &str,
  handler: F,
  signal: S,
) where
  F: Fn(WebTransportSession) -> Pin<Box<dyn Future<Output = ()> + Send>>
    + Send
    + Sync
    + 'static,
  S: Future<Output = ()> + Send + 'static,
{
  if let Err(e) = run(addr, cert_path, key_path, handler, Some(signal)).await {
    tracing::error!("WebTransport server error: {e}");
  }
}

async fn run<F>(
  addr: &str,
  cert_path: &str,
  key_path: &str,
  handler: F,
  signal: Option<impl Future<Output = ()>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
  F: Fn(WebTransportSession) -> Pin<Box<dyn Future<Output = ()> + Send>>
    + Send
    + Sync
    + 'static,
{
  // Load certs using the same helpers as server_h3
  let _ = rustls::crypto::ring::default_provider().install_default();

  let certs = tako_server::server_h3::load_certs(cert_path)?;
  let key = tako_server::server_h3::load_key(key_path)?;

  let mut tls_config = rustls::ServerConfig::builder()
    .with_no_client_auth()
    .with_single_cert(certs, key)?;

  tls_config.max_early_data_size = u32::MAX;
  tls_config.alpn_protocols = vec![b"h3".to_vec()];

  let mut server_config =
    quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls_config)?));

  // Enable datagrams for WebTransport
  let mut transport_config = quinn::TransportConfig::default();
  transport_config.datagram_receive_buffer_size(Some(65536));
  transport_config.max_idle_timeout(Some(Duration::from_secs(30).try_into().unwrap()));
  server_config.transport_config(Arc::new(transport_config));

  let socket_addr: SocketAddr = addr.parse()?;
  let endpoint = quinn::Endpoint::server(server_config, socket_addr)?;

  tracing::info!("WebTransport server listening on {}", endpoint.local_addr()?);

  let handler = Arc::new(handler);
  let mut join_set = tokio::task::JoinSet::new();

  let signal = signal.map(|s| Box::pin(s));
  let signal_fused = async {
    if let Some(s) = signal {
      s.await;
    } else {
      std::future::pending::<()>().await;
    }
  };
  tokio::pin!(signal_fused);

  loop {
    tokio::select! {
      maybe_conn = endpoint.accept() => {
        let Some(incoming) = maybe_conn else { break };
        let handler = Arc::clone(&handler);

        join_set.spawn(async move {
          match incoming.await {
            Ok(conn) => {
              let remote = conn.remote_address();
              tracing::debug!("WebTransport connection from {remote}");
              let session = WebTransportSession::new(conn);
              handler(session).await;
              tracing::debug!("WebTransport session closed: {remote}");
            }
            Err(e) => {
              tracing::error!("QUIC connection failed: {e}");
            }
          }
        });
      }
      () = &mut signal_fused => {
        tracing::info!("WebTransport server shutting down...");
        break;
      }
    }
  }

  endpoint.close(quinn::VarInt::from_u32(0), b"server shutting down");

  let drain = tokio::time::timeout(DEFAULT_DRAIN_TIMEOUT, async {
    while join_set.join_next().await.is_some() {}
  });

  if drain.await.is_err() {
    tracing::warn!(
      "Drain timeout exceeded, aborting {} remaining sessions",
      join_set.len()
    );
    join_set.abort_all();
  }

  endpoint.wait_idle().await;
  tracing::info!("WebTransport server shut down gracefully");
  Ok(())
}
