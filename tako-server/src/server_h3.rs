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

use std::fs::File;
use std::future::Future;
use std::io::BufReader;
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
use quinn::crypto::rustls::QuicServerConfig;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::PrivateKeyDer;
use rustls_pemfile::certs;
use rustls_pemfile::private_key;
use tokio_stream::wrappers::ReceiverStream;

use tako_core::body::TakoBody;
use tako_core::router::Router;
#[cfg(feature = "signals")]
use tako_core::signals::Signal;
#[cfg(feature = "signals")]
use tako_core::signals::SignalArbiter;
#[cfg(feature = "signals")]
use tako_core::signals::ids;
use tako_core::types::BoxError;

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
/// Default drain timeout for graceful shutdown (30 seconds).
const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn serve_h3(router: Router, addr: &str, certs: Option<&str>, key: Option<&str>) {
  if let Err(e) = run(router, addr, certs, key, None::<std::future::Pending<()>>).await {
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
  if let Err(e) = run(router, addr, certs, key, Some(signal)).await {
    tracing::error!("HTTP/3 server error: {e}");
  }
}

/// Runs the HTTP/3 server loop.
async fn run(
  router: Router,
  addr: &str,
  certs: Option<&str>,
  key: Option<&str>,
  signal: Option<impl Future<Output = ()>>,
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
    SignalArbiter::emit_app(
      Signal::with_capacity(ids::SERVER_STARTED, 3)
        .meta("addr", addr_str.clone())
        .meta("transport", "quic")
        .meta("protocol", "h3"),
    )
    .await;
  }

  tracing::info!("Tako HTTP/3 listening on {}", addr_str);

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
        let Some(new_conn) = maybe_conn else { break };
        let router = router.clone();

        join_set.spawn(async move {
          match new_conn.await {
            Ok(conn) => {
              let remote_addr = conn.remote_address();

              #[cfg(feature = "signals")]
              {
                SignalArbiter::emit_app(
                  Signal::with_capacity(ids::CONNECTION_OPENED, 2)
                    .meta("remote_addr", remote_addr.to_string())
                    .meta("protocol", "h3"),
                )
                .await;
              }

              if let Err(e) = handle_connection(conn, router, remote_addr).await {
                tracing::error!("HTTP/3 connection error: {e}");
              }

              #[cfg(feature = "signals")]
              {
                SignalArbiter::emit_app(
                  Signal::with_capacity(ids::CONNECTION_CLOSED, 2)
                    .meta("remote_addr", remote_addr.to_string())
                    .meta("protocol", "h3"),
                )
                .await;
              }
            }
            Err(e) => {
              tracing::error!("QUIC connection failed: {e}");
            }
          }
        });
      }
      () = &mut signal_fused => {
        tracing::info!("Shutdown signal received, draining HTTP/3 connections...");
        break;
      }
    }
  }

  // Close the endpoint to stop accepting new connections
  endpoint.close(0u32.into(), b"server shutting down");

  // Drain in-flight connections
  let drain = tokio::time::timeout(DEFAULT_DRAIN_TIMEOUT, async {
    while join_set.join_next().await.is_some() {}
  });

  if drain.await.is_err() {
    tracing::warn!(
      "Drain timeout ({:?}) exceeded, aborting {} remaining HTTP/3 connections",
      DEFAULT_DRAIN_TIMEOUT,
      join_set.len()
    );
    join_set.abort_all();
  }

  endpoint.wait_idle().await;
  tracing::info!("HTTP/3 server shut down gracefully");
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
  #[cfg(feature = "signals")]
  let path = req.uri().path().to_string();
  #[cfg(feature = "signals")]
  let method = req.method().to_string();

  #[cfg(feature = "signals")]
  {
    SignalArbiter::emit_app(
      Signal::with_capacity(ids::REQUEST_STARTED, 3)
        .meta("method", method.clone())
        .meta("path", path.clone())
        .meta("protocol", "h3"),
    )
    .await;
  }

  // Split into send and recv halves so the handler can stream the body while we
  // hold the send half locally for the response.
  let (mut send_stream, recv_stream) = stream.split();

  // Build request with a streaming body (data + trailers).
  let (parts, _) = req.into_parts();
  let body = build_h3_body(recv_stream);
  let mut tako_req = Request::from_parts(parts, body);
  tako_req.extensions_mut().insert(remote_addr);

  // Dispatch through router
  let response = router.dispatch(tako_req).await;

  #[cfg(feature = "signals")]
  {
    SignalArbiter::emit_app(
      Signal::with_capacity(ids::REQUEST_COMPLETED, 4)
        .meta("method", method)
        .meta("path", path)
        .meta("status", response.status().as_u16().to_string())
        .meta("protocol", "h3"),
    )
    .await;
  }

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

/// Loads TLS certificates from a PEM-encoded file.
pub fn load_certs(path: &str) -> anyhow::Result<Vec<CertificateDer<'static>>> {
  let mut rd = BufReader::new(
    File::open(path).map_err(|e| anyhow::anyhow!("failed to open cert file '{}': {}", path, e))?,
  );
  certs(&mut rd)
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| anyhow::anyhow!("failed to parse certs from '{}': {}", path, e))
}

/// Loads a private key from a PEM-encoded file.
///
/// Accepts PKCS#8, PKCS#1 (RSA) and SEC1 (EC) PEM blocks.
pub fn load_key(path: &str) -> anyhow::Result<PrivateKeyDer<'static>> {
  let mut rd = BufReader::new(
    File::open(path).map_err(|e| anyhow::anyhow!("failed to open key file '{}': {}", path, e))?,
  );
  private_key(&mut rd)
    .map_err(|e| anyhow::anyhow!("bad private key in '{}': {}", path, e))?
    .ok_or_else(|| anyhow::anyhow!("no PEM private key (PKCS#8, PKCS#1 or SEC1) found in '{}'", path))
}
