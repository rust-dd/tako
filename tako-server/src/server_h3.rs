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

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Buf;
use bytes::Bytes;
use bytes::BytesMut;
use h3::quic::BidiStream;
use h3::quic::RecvStream;
use h3::server::RequestStream;
use http::HeaderMap;
use http::Request;
use http_body::Body;
use http_body::Frame;
use quinn::VarInt;
use quinn::congestion::{BbrConfig, CubicConfig, NewRenoConfig};
use quinn::crypto::rustls::QuicServerConfig;
use tokio::sync::Notify;
use tokio_stream::wrappers::ReceiverStream;

use tako_core::body::TakoBody;
use tako_core::conn_info::{ConnInfo, TlsInfo};
use tako_core::router::Router;
#[cfg(feature = "signals")]
use tako_core::signals::transport as signal_tx;
use tako_core::types::BoxError;

use crate::{H3Congestion, ServerConfig};

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
  if let Err(e) = run(
    router,
    addr,
    certs,
    key,
    None::<std::future::Pending<()>>,
    ServerConfig::default(),
  )
  .await
  {
    tracing::error!("HTTP/3 server error: {e}");
  }
}

/// Starts an HTTP/3 server with graceful shutdown support.
pub async fn serve_h3_with_shutdown(
  router: Router,
  addr: &str,
  certs: Option<&str>,
  key: Option<&str>,
  signal: impl Future<Output = ()>,
) {
  if let Err(e) = run(router, addr, certs, key, Some(signal), ServerConfig::default()).await {
    tracing::error!("HTTP/3 server error: {e}");
  }
}

/// Like [`serve_h3`] with caller-supplied [`ServerConfig`].
pub async fn serve_h3_with_config(
  router: Router,
  addr: &str,
  certs: Option<&str>,
  key: Option<&str>,
  config: ServerConfig,
) {
  if let Err(e) = run(
    router,
    addr,
    certs,
    key,
    None::<std::future::Pending<()>>,
    config,
  )
  .await
  {
    tracing::error!("HTTP/3 server error: {e}");
  }
}

/// Like [`serve_h3_with_shutdown`] with caller-supplied [`ServerConfig`].
pub async fn serve_h3_with_shutdown_and_config(
  router: Router,
  addr: &str,
  certs: Option<&str>,
  key: Option<&str>,
  signal: impl Future<Output = ()>,
  config: ServerConfig,
) {
  if let Err(e) = run(router, addr, certs, key, Some(signal), config).await {
    tracing::error!("HTTP/3 server error: {e}");
  }
}

/// Run an HTTP/3 server with a caller-built `Arc<rustls::ServerConfig>`. The
/// caller is responsible for setting `alpn_protocols = [b"h3"]` (the
/// [`crate::build_rustls_server_config`] helper does this when given the right
/// ALPN list). Use this when constructing the TLS config via [`crate::TlsCert`]
/// variants beyond `PemPaths`.
pub async fn serve_h3_with_rustls_config(
  router: Router,
  addr: &str,
  rustls_config: Arc<rustls::ServerConfig>,
  config: ServerConfig,
) {
  if let Err(e) = run_with_rustls_config(
    router,
    addr,
    rustls_config,
    None::<std::future::Pending<()>>,
    config,
  )
  .await
  {
    tracing::error!("HTTP/3 server error: {e}");
  }
}

/// Like [`serve_h3_with_rustls_config`] with graceful shutdown.
pub async fn serve_h3_with_rustls_config_and_shutdown(
  router: Router,
  addr: &str,
  rustls_config: Arc<rustls::ServerConfig>,
  signal: impl Future<Output = ()>,
  config: ServerConfig,
) {
  if let Err(e) =
    run_with_rustls_config(router, addr, rustls_config, Some(signal), config).await
  {
    tracing::error!("HTTP/3 server error: {e}");
  }
}

/// Build a `quinn::TransportConfig` from the H3-specific knobs in [`ServerConfig`].
fn transport_config_from(config: &ServerConfig) -> quinn::TransportConfig {
  let mut tc = quinn::TransportConfig::default();
  tc.max_concurrent_bidi_streams(VarInt::from_u32(config.h3_max_concurrent_bidi_streams));
  tc.max_concurrent_uni_streams(VarInt::from_u32(config.h3_max_concurrent_uni_streams));
  if let Some(idle) = config.h3_max_idle_timeout {
    if let Ok(idle) = idle.try_into() {
      tc.max_idle_timeout(Some(idle));
    }
  }
  // QUIC datagrams (RFC 9221). Required for downstream WebTransport-style
  // traffic. Send buffer is left at the quinn default.
  if config.h3_enable_datagrams {
    tc.datagram_receive_buffer_size(Some(64 * 1024));
  } else {
    tc.datagram_receive_buffer_size(None);
  }
  match config.h3_congestion {
    H3Congestion::Cubic => {
      tc.congestion_controller_factory(Arc::new(CubicConfig::default()));
    }
    H3Congestion::NewReno => {
      tc.congestion_controller_factory(Arc::new(NewRenoConfig::default()));
    }
    H3Congestion::Bbr => {
      tc.congestion_controller_factory(Arc::new(BbrConfig::default()));
    }
  }
  tc
}

/// Runs the HTTP/3 server loop.
async fn run(
  router: Router,
  addr: &str,
  certs: Option<&str>,
  key: Option<&str>,
  signal: Option<impl Future<Output = ()>>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  #[cfg(feature = "tako-tracing")]
  tako_core::tracing::init_tracing();

  // Install default crypto provider for rustls (required for QUIC/TLS)
  let _ = rustls::crypto::ring::default_provider().install_default();

  let certs_vec = load_certs(certs.unwrap_or("cert.pem"))?;
  let key = load_key(key.unwrap_or("key.pem"))?;

  let mut tls_config = rustls::ServerConfig::builder()
    .with_no_client_auth()
    .with_single_cert(certs_vec, key)?;

  // 0-RTT (early data) is disabled by default: the server has no replay-protection
  // wiring on the request path, so accepting early-data application bytes would
  // expose idempotent endpoints to replay attacks. Re-enabling requires plumbing a
  // replay cache and a typed extractor — see V2_ROADMAP.md § 1.5.
  tls_config.max_early_data_size = 0;
  tls_config.alpn_protocols = vec![b"h3".to_vec()];

  run_with_rustls_config(router, addr, Arc::new(tls_config), signal, config).await
}

/// Variant of [`run`] that accepts a pre-built `Arc<rustls::ServerConfig>`.
async fn run_with_rustls_config(
  router: Router,
  addr: &str,
  tls_config: Arc<rustls::ServerConfig>,
  signal: Option<impl Future<Output = ()>>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  #[cfg(feature = "tako-tracing")]
  tako_core::tracing::init_tracing();

  // Install default crypto provider for rustls (required for QUIC/TLS)
  let _ = rustls::crypto::ring::default_provider().install_default();

  // QuicServerConfig wraps a rustls::ServerConfig; it requires the underlying
  // config to set ALPN to h3. Calling `try_from` errors otherwise, so we trust
  // the caller (or the build_rustls_server_config helper) to pre-set ALPN.
  let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(
    QuicServerConfig::try_from((*tls_config).clone())?,
  ));
  server_config.transport_config(Arc::new(transport_config_from(&config)));

  let socket_addr: SocketAddr = addr.parse()?;
  let endpoint = quinn::Endpoint::server(server_config, socket_addr)?;

  let router = Arc::new(router);

  #[cfg(feature = "plugins")]
  router.setup_plugins_once();

  let addr_str = endpoint.local_addr()?.to_string();

  #[cfg(feature = "signals")]
  signal_tx::emit_server_started(&addr_str, "quic", true).await;

  tracing::info!("Tako HTTP/3 listening on {}", addr_str);

  let mut join_set = tokio::task::JoinSet::new();
  let drain_timeout = config.drain_timeout;
  let goaway_grace = config.h3_goaway_grace.min(drain_timeout);
  let h3_use_retry = config.h3_use_retry;
  let max_conn_semaphore = config.max_connections.map(|n| Arc::new(tokio::sync::Semaphore::new(n)));

  // Per-connection graceful shutdown: when triggered, every spawned connection
  // task races its `accept()` against this notify, then calls `h3.shutdown(0)`
  // so the peer sees a GOAWAY frame before the QUIC endpoint hard-closes.
  let conn_shutdown = Arc::new(Notify::new());

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

        // Optional address-validation retry. Defends against UDP source-IP
        // spoofing amplification by forcing the client through one extra
        // round-trip with a server-issued retry token.
        if h3_use_retry && !incoming.remote_address_validated() {
          if let Err(e) = incoming.retry() {
            tracing::debug!("HTTP/3 retry refused: {e}");
          }
          continue;
        }

        let permit = if let Some(sem) = &max_conn_semaphore {
          match sem.clone().acquire_owned().await {
            Ok(p) => Some(p),
            Err(_) => continue,
          }
        } else {
          None
        };
        let router = router.clone();
        let conn_shutdown = conn_shutdown.clone();

        join_set.spawn(async move {
          match incoming.await {
            Ok(conn) => {
              let remote_addr = conn.remote_address();

              #[cfg(feature = "signals")]
              signal_tx::emit_connection_opened(&remote_addr.to_string(), true, Some("h3")).await;

              if let Err(e) =
                handle_connection(conn, router, remote_addr, conn_shutdown, goaway_grace).await
              {
                tracing::error!("HTTP/3 connection error: {e}");
              }

              #[cfg(feature = "signals")]
              signal_tx::emit_connection_closed(&remote_addr.to_string(), true, Some("h3")).await;
            }
            Err(e) => {
              tracing::error!("QUIC connection failed: {e}");
            }
          }

          drop(permit);
        });
      }
      () = &mut signal_fused => {
        tracing::info!("Shutdown signal received, sending HTTP/3 GOAWAY...");
        break;
      }
    }
  }

  // Phase 1: kick every connection into graceful-shutdown mode so it stops
  // accepting new requests and emits a GOAWAY frame to the peer.
  conn_shutdown.notify_waiters();

  // Phase 2: wait for in-flight connections to finish gracefully, bounded by
  // the global drain deadline.
  let drain = tokio::time::timeout(drain_timeout, async {
    while join_set.join_next().await.is_some() {}
  });

  if drain.await.is_err() {
    tracing::warn!(
      "Drain timeout ({:?}) exceeded, aborting {} remaining HTTP/3 connections",
      drain_timeout,
      join_set.len()
    );
    join_set.abort_all();
  }

  // Phase 3: close the endpoint after grace expired (or all conns settled).
  endpoint.close(0u32.into(), b"server shutting down");
  endpoint.wait_idle().await;
  tracing::info!("HTTP/3 server shut down gracefully");
  Ok(())
}

/// Handles a single HTTP/3 connection.
///
/// Races `accept()` against the per-connection shutdown notify; on shutdown,
/// emits a GOAWAY frame via `h3_conn.shutdown(0)` and waits up to `goaway_grace`
/// for any already-spawned request handlers to finish before returning.
async fn handle_connection(
  conn: quinn::Connection,
  router: Arc<Router>,
  remote_addr: SocketAddr,
  shutdown: Arc<Notify>,
  goaway_grace: Duration,
) -> Result<(), BoxError> {
  let mut h3_conn = h3::server::Connection::new(h3_quinn::Connection::new(conn)).await?;
  let mut request_tasks = tokio::task::JoinSet::new();
  let shutdown_notified = shutdown.notified();
  tokio::pin!(shutdown_notified);

  loop {
    tokio::select! {
      accepted = h3_conn.accept() => {
        match accepted {
          Ok(Some(resolver)) => {
            let router = router.clone();
            request_tasks.spawn(async move {
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
          Ok(None) => break,
          Err(e) => {
            tracing::error!("HTTP/3 accept error: {e}");
            break;
          }
        }
      }
      () = shutdown_notified.as_mut() => {
        // Send GOAWAY(0): the peer must not start any new request, but we
        // continue draining streams already in flight on this connection.
        if let Err(e) = h3_conn.shutdown(0).await {
          tracing::debug!("HTTP/3 GOAWAY error: {e}");
        }
        break;
      }
    }
  }

  // Drain in-flight request handlers within the per-connection grace.
  let drain = tokio::time::timeout(goaway_grace, async {
    while request_tasks.join_next().await.is_some() {}
  });
  if drain.await.is_err() {
    tracing::debug!(
      "HTTP/3 connection grace ({:?}) elapsed; aborting {} request task(s)",
      goaway_grace,
      request_tasks.len()
    );
    request_tasks.abort_all();
  }

  Ok(())
}

/// Channel buffer for the H3 streaming body.
///
/// Bounds the number of in-flight frames between the QUIC receiver task and the
/// handler so that a slow handler exerts backpressure on the client instead of
/// growing memory unboundedly.
const H3_BODY_CHANNEL_CAPACITY: usize = 8;

/// Builds a streaming `TakoBody` backed by an HTTP/3 receive stream.
///
/// Spawns a forwarder task that pulls QUIC chunks via `recv_data`, emits them as
/// `Frame::data`, and then pulls trailers via `recv_trailers` to emit a
/// `Frame::trailers`. The bounded channel provides natural backpressure.
fn build_h3_body<R>(mut recv: RequestStream<R, Bytes>) -> TakoBody
where
  R: RecvStream + Send + 'static,
{
  let (tx, rx) = tokio::sync::mpsc::channel::<Result<Frame<Bytes>, BoxError>>(H3_BODY_CHANNEL_CAPACITY);
  tokio::spawn(async move {
    loop {
      match recv.recv_data().await {
        Ok(Some(mut chunk)) => {
          let mut buf = BytesMut::with_capacity(chunk.remaining());
          while chunk.has_remaining() {
            let slice = chunk.chunk();
            buf.extend_from_slice(slice);
            let len = slice.len();
            chunk.advance(len);
          }
          if !buf.is_empty() && tx.send(Ok(Frame::data(buf.freeze()))).await.is_err() {
            return;
          }
        }
        Ok(None) => break,
        Err(e) => {
          let _ = tx.send(Err(Box::new(e) as BoxError)).await;
          return;
        }
      }
    }
    match recv.recv_trailers().await {
      Ok(Some(trailers)) => {
        let _ = tx.send(Ok(Frame::trailers(trailers))).await;
      }
      Ok(None) => {}
      Err(e) => {
        let _ = tx.send(Err(Box::new(e) as BoxError)).await;
      }
    }
  });

  TakoBody::from_try_stream(ReceiverStream::new(rx))
}

/// Handles a single HTTP/3 request.
async fn handle_request<S>(
  req: Request<()>,
  stream: RequestStream<S, Bytes>,
  router: Arc<Router>,
  remote_addr: SocketAddr,
) -> Result<(), BoxError>
where
  S: BidiStream<Bytes> + Send + 'static,
  <S as BidiStream<Bytes>>::SendStream: Send + 'static,
  <S as BidiStream<Bytes>>::RecvStream: Send + 'static,
{
  // Per-request signals fire from inside Router::dispatch.

  // Split into send and recv halves so the handler can stream the body while we
  // hold the send half locally for the response.
  let (mut send_stream, recv_stream) = stream.split();

  // Build request with a streaming body (data + trailers).
  let (parts, _) = req.into_parts();
  let body = build_h3_body(recv_stream);
  let mut tako_req = Request::from_parts(parts, body);
  tako_req.extensions_mut().insert(remote_addr);
  tako_req.extensions_mut().insert(ConnInfo::h3(
    remote_addr,
    TlsInfo {
      alpn: Some(b"h3".to_vec()),
      sni: None,
      version: Some("TLSv1.3"),
    },
  ));

  // Dispatch through router
  let response = router.dispatch(tako_req).await;

  // Send response head
  let (parts, body) = response.into_parts();
  let resp = http::Response::from_parts(parts, ());
  send_stream.send_response(resp).await?;

  // Stream response body frame by frame; preserve trailers through to send_trailers.
  let mut body = std::pin::pin!(body);
  let mut response_trailers: Option<HeaderMap> = None;
  while let Some(frame_res) = std::future::poll_fn(|cx| body.as_mut().poll_frame(cx)).await {
    match frame_res {
      Ok(frame) => {
        if frame.is_data() {
          if let Ok(data) = frame.into_data()
            && !data.is_empty()
          {
            send_stream.send_data(data).await?;
          }
        } else if frame.is_trailers() {
          if let Ok(t) = frame.into_trailers() {
            // Last trailer frame wins; HTTP responses are not expected to emit multiple.
            response_trailers = Some(t);
          }
        }
      }
      Err(e) => {
        tracing::error!("HTTP/3 body frame error: {e}");
        break;
      }
    }
  }

  if let Some(trailers) = response_trailers {
    send_stream.send_trailers(trailers).await?;
  } else {
    send_stream.finish().await?;
  }

  Ok(())
}

/// Loads TLS certificates from a PEM-encoded file. Re-export of
/// [`tako_core::tls::load_certs`].
pub use tako_core::tls::load_certs;

/// Loads a private key from a PEM-encoded file. Accepts PKCS#8, PKCS#1 (RSA),
/// and SEC1 (EC) PEM blocks. Re-export of [`tako_core::tls::load_key`].
pub use tako_core::tls::load_key;
