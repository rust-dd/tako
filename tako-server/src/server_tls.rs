#![cfg(feature = "tls")]
#![cfg_attr(docsrs, doc(cfg(feature = "tls")))]

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

use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;

use hyper::server::conn::http1;
#[cfg(feature = "http2")]
use hyper::server::conn::http2;
use hyper::service::service_fn;
#[cfg(feature = "http2")]
use hyper_util::rt::TokioExecutor;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig as RustlsServerConfig;

use tako_core::body::TakoBody;
use tako_core::conn_info::{ConnInfo, TlsInfo};
use tako_core::router::Router;
#[cfg(feature = "signals")]
use tako_core::signals::transport as signal_tx;
use tako_core::types::BoxError;

use crate::ServerConfig;

// HTTP/2 hardening + connection lifetimes are sourced from `ServerConfig`,
// whose `Default` mirrors the historical hardcoded values (30 s drain, 100
// streams, 16 KiB header list, 1 MiB send buf, 50 pending-accept resets).
//
// Pass a custom [`ServerConfig`] via [`serve_tls_with_config`] /
// [`serve_tls_with_shutdown_and_config`] to override individual knobs while
// keeping perf-neutral defaults for everything you don't touch.

/// Starts a TLS-enabled HTTP server with the given listener, router, and certificates.
pub async fn serve_tls(
  listener: TcpListener,
  router: Router,
  certs: Option<&str>,
  key: Option<&str>,
) {
  if let Err(e) = run(
    listener,
    router,
    certs,
    key,
    None::<std::future::Pending<()>>,
    ServerConfig::default(),
  )
  .await
  {
    tracing::error!("TLS server error: {e}");
  }
}

/// Starts a TLS-enabled HTTP server with graceful shutdown support.
pub async fn serve_tls_with_shutdown(
  listener: TcpListener,
  router: Router,
  certs: Option<&str>,
  key: Option<&str>,
  signal: impl Future<Output = ()>,
) {
  if let Err(e) = run(
    listener,
    router,
    certs,
    key,
    Some(signal),
    ServerConfig::default(),
  )
  .await
  {
    tracing::error!("TLS server error: {e}");
  }
}

/// Like [`serve_tls`] but with caller-supplied [`ServerConfig`].
pub async fn serve_tls_with_config(
  listener: TcpListener,
  router: Router,
  certs: Option<&str>,
  key: Option<&str>,
  config: ServerConfig,
) {
  if let Err(e) = run(
    listener,
    router,
    certs,
    key,
    None::<std::future::Pending<()>>,
    config,
  )
  .await
  {
    tracing::error!("TLS server error: {e}");
  }
}

/// Like [`serve_tls_with_shutdown`] but with caller-supplied [`ServerConfig`].
pub async fn serve_tls_with_shutdown_and_config(
  listener: TcpListener,
  router: Router,
  certs: Option<&str>,
  key: Option<&str>,
  signal: impl Future<Output = ()>,
  config: ServerConfig,
) {
  if let Err(e) = run(listener, router, certs, key, Some(signal), config).await {
    tracing::error!("TLS server error: {e}");
  }
}

/// Run with a caller-built `Arc<rustls::ServerConfig>`. Use this when
/// constructing the TLS config via [`crate::TlsCert`] variants beyond
/// `PemPaths` (`Der`, `Resolver`, mTLS) — the [`crate::build_rustls_server_config`]
/// helper handles the assembly.
pub async fn serve_tls_with_rustls_config(
  listener: TcpListener,
  router: Router,
  rustls_config: Arc<RustlsServerConfig>,
  config: ServerConfig,
) {
  if let Err(e) = run_with_config(
    listener,
    router,
    rustls_config,
    None::<std::future::Pending<()>>,
    config,
  )
  .await
  {
    tracing::error!("TLS server error: {e}");
  }
}

/// Like [`serve_tls_with_rustls_config`] with graceful shutdown.
pub async fn serve_tls_with_rustls_config_and_shutdown(
  listener: TcpListener,
  router: Router,
  rustls_config: Arc<RustlsServerConfig>,
  signal: impl Future<Output = ()>,
  config: ServerConfig,
) {
  if let Err(e) = run_with_config(listener, router, rustls_config, Some(signal), config).await {
    tracing::error!("TLS server error: {e}");
  }
}

/// Runs the TLS server loop, handling secure connections and request dispatch.
pub async fn run(
  listener: TcpListener,
  router: Router,
  certs: Option<&str>,
  key: Option<&str>,
  signal: Option<impl Future<Output = ()>>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  #[cfg(feature = "tako-tracing")]
  tako_core::tracing::init_tracing();

  let certs = load_certs(certs.unwrap_or("cert.pem"))?;
  let key = load_key(key.unwrap_or("key.pem"))?;

  let mut tls_config = RustlsServerConfig::builder()
    .with_no_client_auth()
    .with_single_cert(certs, key)?;

  #[cfg(feature = "http2")]
  {
    tls_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
  }

  #[cfg(not(feature = "http2"))]
  {
    tls_config.alpn_protocols = vec![b"http/1.1".to_vec()];
  }

  run_with_config(listener, router, Arc::new(tls_config), signal, config).await
}

/// Variant of [`run`] that accepts a pre-built `Arc<rustls::ServerConfig>`.
pub async fn run_with_config(
  listener: TcpListener,
  router: Router,
  tls_config: Arc<RustlsServerConfig>,
  signal: Option<impl Future<Output = ()>>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  #[cfg(feature = "tako-tracing")]
  tako_core::tracing::init_tracing();

  let acceptor = TlsAcceptor::from(tls_config);
  let router = Arc::new(router);

  // Setup plugins
  #[cfg(feature = "plugins")]
  router.setup_plugins_once();

  let addr_str = listener.local_addr()?.to_string();

  #[cfg(feature = "signals")]
  signal_tx::emit_server_started(&addr_str, "tcp", true).await;

  tracing::info!("Tako TLS listening on {}", addr_str);

  let mut join_set = JoinSet::new();
  let mut accept_backoff = config.accept_backoff;
  let max_conn_semaphore = config.max_connections.map(|n| Arc::new(tokio::sync::Semaphore::new(n)));
  let drain_timeout = config.drain_timeout;
  let header_read_timeout = config.header_read_timeout;
  let keep_alive = config.keep_alive;
  #[cfg(feature = "http2")]
  let h2_max_concurrent_streams = config.h2_max_concurrent_streams;
  #[cfg(feature = "http2")]
  let h2_max_header_list_size = config.h2_max_header_list_size;
  #[cfg(feature = "http2")]
  let h2_max_send_buf_size = config.h2_max_send_buf_size;
  #[cfg(feature = "http2")]
  let h2_max_pending_accept_reset_streams = config.h2_max_pending_accept_reset_streams;
  #[cfg(feature = "http2")]
  let h2_keep_alive_interval = config.h2_keep_alive_interval;
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
      result = listener.accept() => {
        let (stream, addr) = match result {
          Ok(v) => { accept_backoff.reset(); v }
          Err(err) => {
            tracing::warn!("TLS accept failed: {err}; backing off");
            accept_backoff.sleep_and_grow().await;
            continue;
          }
        };
        let permit = if let Some(sem) = &max_conn_semaphore {
          match sem.clone().acquire_owned().await {
            Ok(p) => Some(p),
            Err(_) => continue,
          }
        } else {
          None
        };
        let _ = stream.set_nodelay(true);
        let acceptor = acceptor.clone();
        let router = router.clone();

        join_set.spawn(async move {
          let tls_stream = match acceptor.accept(stream).await {
            Ok(s) => s,
            Err(e) => {
              tracing::error!("TLS error: {e}");
              return;
            }
          };

          #[cfg(feature = "signals")]
          signal_tx::emit_connection_opened(&addr.to_string(), true, None).await;

          // Capture TLS metadata once per connection so each request can read
          // the same ALPN / SNI / version without touching the live session.
          let alpn_proto = tls_stream.get_ref().1.alpn_protocol().map(<[u8]>::to_vec);
          let sni = tls_stream
            .get_ref()
            .1
            .server_name()
            .map(str::to_string);
          let tls_info = TlsInfo {
            alpn: alpn_proto.clone(),
            sni,
            version: None,
          };
          let is_h2 = matches!(alpn_proto.as_deref(), Some(b"h2"));
          let conn_info = if is_h2 {
            ConnInfo::h2_tls(addr, tls_info)
          } else {
            ConnInfo::h1_tls(addr, tls_info)
          };

          #[cfg(feature = "http2")]
          let proto = alpn_proto;

          let io = TokioIo::new(tls_stream);
          // Per-request signals fire from inside Router::dispatch.
          let svc = service_fn(move |mut req| {
            let r = router.clone();
            let conn_info = conn_info.clone();
            async move {
              req.extensions_mut().insert(addr);
              req.extensions_mut().insert(conn_info);
              let response = r.dispatch(req.map(TakoBody::incoming)).await;
              Ok::<_, Infallible>(response)
            }
          });

          #[cfg(feature = "http2")]
          if proto.as_deref() == Some(b"h2") {
            let mut h2 = http2::Builder::new(TokioExecutor::new());
            h2.max_concurrent_streams(h2_max_concurrent_streams)
              .max_header_list_size(h2_max_header_list_size)
              .max_send_buf_size(h2_max_send_buf_size)
              .max_pending_accept_reset_streams(h2_max_pending_accept_reset_streams);
            if let Some(interval) = h2_keep_alive_interval {
              h2.keep_alive_interval(Some(interval));
            }

            if let Err(e) = h2.serve_connection(io, svc).await {
              tracing::error!("HTTP/2 error: {e}");
            }

            #[cfg(feature = "signals")]
            signal_tx::emit_connection_closed(&addr.to_string(), true, None).await;
            return;
          }

          let mut h1 = http1::Builder::new();
          h1.keep_alive(keep_alive);
          h1.timer(hyper_util::rt::TokioTimer::new());
          if let Some(t) = header_read_timeout {
            h1.header_read_timeout(t);
          }

          if let Err(e) = h1.serve_connection(io, svc).with_upgrades().await {
            if e.is_incomplete_message() {
              tracing::debug!("TLS HTTP/1.1 client disconnected mid-message: {e}");
            } else {
              tracing::error!("HTTP/1.1 error: {e}");
            }
          }

          #[cfg(feature = "signals")]
          signal_tx::emit_connection_closed(&addr.to_string(), true, None).await;

          drop(permit);
        });
      }
      () = &mut signal_fused => {
        tracing::info!("Shutdown signal received, draining TLS connections...");
        break;
      }
    }
  }

  // Drain in-flight connections
  let drain = tokio::time::timeout(drain_timeout, async {
    while join_set.join_next().await.is_some() {}
  });

  if drain.await.is_err() {
    tracing::warn!(
      "Drain timeout ({:?}) exceeded, aborting {} remaining TLS connections",
      drain_timeout,
      join_set.len()
    );
    join_set.abort_all();
  }

  tracing::info!("TLS server shut down gracefully");
  Ok(())
}

/// Loads TLS certificates from a PEM-encoded file.
///
/// Thin re-export of [`tako_core::tls::load_certs`]; preserved for backward
/// compatibility.
pub use tako_core::tls::load_certs;

/// Loads a private key from a PEM-encoded file.
///
/// Accepts PKCS#8, PKCS#1 (RSA), and SEC1 (EC) PEM blocks. Thin re-export of
/// [`tako_core::tls::load_key`]; preserved for backward compatibility.
pub use tako_core::tls::load_key;
