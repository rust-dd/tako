//! Raw QUIC sessions for peers that do not speak WebTransport; browsers need the W3C
//! `WebTransport` extractor on the HTTP/3 server instead.
//!
//! # Examples
//!
//! ```rust,no_run
//! # #[cfg(feature = "webtransport")]
//! use tako::webtransport::{serve_webtransport, RawQuicSession};
//!
//! # #[cfg(feature = "webtransport")]
//! # async fn example() {
//! serve_webtransport("[::]:4433", "cert.pem", "key.pem", |session: RawQuicSession| {
//!     Box::pin(async move {
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

/// How long shutdown waits for sessions before aborting them.
const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

/// A QUIC connection with datagrams enabled.
pub struct RawQuicSession {
  conn: quinn::Connection,
}

impl RawQuicSession {
  pub fn new(conn: quinn::Connection) -> Self {
    Self { conn }
  }

  pub fn remote_address(&self) -> SocketAddr {
    self.conn.remote_address()
  }

  pub async fn accept_bi(
    &self,
  ) -> Result<(quinn::SendStream, quinn::RecvStream), quinn::ConnectionError> {
    self.conn.accept_bi().await
  }

  pub async fn accept_uni(&self) -> Result<quinn::RecvStream, quinn::ConnectionError> {
    self.conn.accept_uni().await
  }

  pub async fn open_bi(
    &self,
  ) -> Result<(quinn::SendStream, quinn::RecvStream), quinn::ConnectionError> {
    self.conn.open_bi().await
  }

  pub async fn open_uni(&self) -> Result<quinn::SendStream, quinn::ConnectionError> {
    self.conn.open_uni().await
  }

  pub async fn read_datagram(&self) -> Result<bytes::Bytes, quinn::ConnectionError> {
    self.conn.read_datagram().await
  }

  pub fn send_datagram(&self, data: bytes::Bytes) -> Result<(), quinn::SendDatagramError> {
    self.conn.send_datagram(data)
  }

  pub fn close(&self, code: u32, reason: &[u8]) {
    self.conn.close(quinn::VarInt::from_u32(code), reason);
  }

  pub fn connection(&self) -> &quinn::Connection {
    &self.conn
  }
}

/// Runs a QUIC server with ALPN `h3` until the endpoint closes; errors are logged, not returned.
pub async fn serve_webtransport<F>(addr: &str, cert_path: &str, key_path: &str, handler: F)
where
  F: Fn(RawQuicSession) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static,
{
  if let Err(e) = run(
    addr,
    cert_path,
    key_path,
    handler,
    None::<std::future::Pending<()>>,
  )
  .await
  {
    tracing::error!("WebTransport server error: {e}");
  }
}

/// [`serve_webtransport`] until `signal` resolves, then drains sessions for up to 30 seconds.
pub async fn serve_webtransport_with_shutdown<F, S>(
  addr: &str,
  cert_path: &str,
  key_path: &str,
  handler: F,
  signal: S,
) where
  F: Fn(RawQuicSession) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static,
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
  F: Fn(RawQuicSession) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static,
{
  let _ = rustls::crypto::ring::default_provider().install_default();

  let certs = tako_rs_core::tls::load_certs(cert_path)?;
  let key = tako_rs_core::tls::load_key(key_path)?;

  let mut tls_config = rustls::ServerConfig::builder()
    .with_no_client_auth()
    .with_single_cert(certs, key)?;

  tls_config.max_early_data_size = u32::MAX;
  tls_config.alpn_protocols = vec![b"h3".to_vec()];

  let mut server_config =
    quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls_config)?));

  let mut transport_config = quinn::TransportConfig::default();
  transport_config.datagram_receive_buffer_size(Some(65536));
  transport_config.max_idle_timeout(Some(Duration::from_secs(30).try_into().unwrap()));
  server_config.transport_config(Arc::new(transport_config));

  let socket_addr: SocketAddr = addr.parse()?;
  let endpoint = quinn::Endpoint::server(server_config, socket_addr)?;

  tracing::info!(
    "WebTransport server listening on {}",
    endpoint.local_addr()?
  );

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
              let session = RawQuicSession::new(conn);
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
    let remaining = join_set.len();
    tracing::warn!("Drain timeout exceeded, aborting {remaining} remaining sessions");
    join_set.abort_all();
  }

  endpoint.wait_idle().await;
  tracing::info!("WebTransport server shut down gracefully");
  Ok(())
}
