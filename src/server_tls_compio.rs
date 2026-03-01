#![cfg(feature = "tls")]
#![cfg_attr(docsrs, doc(cfg(feature = "tls")))]

//! TLS-enabled HTTP server implementation for secure connections (compio runtime).

#[cfg(feature = "signals")]
use std::collections::HashMap;
use std::convert::Infallible;
use std::fs::File;
use std::future::Future;
use std::io::BufReader;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use compio::net::TcpListener;
use compio::tls::TlsAcceptor;
use cyper_core::HyperStream;
use futures_util::future::Either;
use hyper::server::conn::http1;
#[cfg(feature = "http2")]
use hyper::server::conn::http2;
use hyper::service::service_fn;
#[cfg(feature = "http2")]
use hyper_util::rt::TokioExecutor;
use rustls::ServerConfig;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::PrivateKeyDer;
use rustls_pemfile::certs;
use rustls_pemfile::pkcs8_private_keys;
use tokio::sync::Notify;

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

/// Default drain timeout for graceful shutdown (30 seconds).
const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

/// Starts a TLS-enabled HTTP server with the given listener, router, and certificates.
pub async fn serve_tls(
  listener: TcpListener,
  router: Router,
  certs: Option<&str>,
  key: Option<&str>,
) {
  if let Err(e) = run(listener, router, certs, key, None::<std::future::Pending<()>>).await {
    tracing::error!("TLS server error: {e}");
  }
}

/// Starts a TLS-enabled HTTP server (compio) with graceful shutdown support.
pub async fn serve_tls_with_shutdown(
  listener: TcpListener,
  router: Router,
  certs: Option<&str>,
  key: Option<&str>,
  signal: impl Future<Output = ()>,
) {
  if let Err(e) = run(listener, router, certs, key, Some(signal)).await {
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
) -> Result<(), BoxError> {
  #[cfg(feature = "tako-tracing")]
  crate::tracing::init_tracing();

  let certs = load_certs(certs.unwrap_or("cert.pem"))?;
  let key = load_key(key.unwrap_or("key.pem"))?;

  let mut config = ServerConfig::builder()
    .with_no_client_auth()
    .with_single_cert(certs, key)?;

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

  #[cfg(feature = "plugins")]
  router.setup_plugins_once();

  let addr_str = listener.local_addr()?.to_string();

  #[cfg(feature = "signals")]
  {
    let mut server_meta: HashMap<String, String, BuildHasher> =
      HashMap::with_hasher(BuildHasher::default());
    server_meta.insert("addr".to_string(), addr_str.clone());
    server_meta.insert("transport".to_string(), "tcp".to_string());
    server_meta.insert("tls".to_string(), "true".to_string());
    SignalArbiter::emit_app(Signal::with_metadata(ids::SERVER_STARTED, server_meta)).await;
  }

  tracing::info!("Tako TLS listening on {}", addr_str);

  let inflight = Arc::new(AtomicUsize::new(0));
  let drain_notify = Arc::new(Notify::new());

  let signal = signal.map(|s| Box::pin(s));
  let mut signal_fused = std::pin::pin!(async {
    if let Some(s) = signal {
      s.await;
    } else {
      std::future::pending::<()>().await;
    }
  });

  loop {
    let accept = std::pin::pin!(listener.accept());
    match futures_util::future::select(accept, signal_fused.as_mut()).await {
      Either::Left((result, _)) => {
        let (stream, addr) = result?;
        let acceptor = acceptor.clone();
        let router = router.clone();
        let inflight = inflight.clone();
        let drain_notify = drain_notify.clone();

        inflight.fetch_add(1, Ordering::SeqCst);

        compio::runtime::spawn(async move {
          let tls_stream = match acceptor.accept(stream).await {
            Ok(s) => s,
            Err(e) => {
              tracing::error!("TLS error: {e}");
              if inflight.fetch_sub(1, Ordering::SeqCst) == 1 {
                drain_notify.notify_one();
              }
              return;
            }
          };

          #[cfg(feature = "signals")]
          {
            let mut conn_open_meta: HashMap<String, String, BuildHasher> =
              HashMap::with_hasher(BuildHasher::default());
            conn_open_meta.insert("remote_addr".to_string(), addr.to_string());
            conn_open_meta.insert("tls".to_string(), "true".to_string());
            SignalArbiter::emit_app(Signal::with_metadata(
              ids::CONNECTION_OPENED,
              conn_open_meta,
            ))
            .await;
          }

          #[cfg(feature = "http2")]
          let proto = tls_stream.negotiated_alpn().map(|p| p.to_vec());

          let io = HyperStream::new(tls_stream);
          let svc = service_fn(move |mut req| {
            let r = router.clone();
            async move {
              #[cfg(feature = "signals")]
              let path = req.uri().path().to_string();
              #[cfg(feature = "signals")]
              let method = req.method().to_string();

              req.extensions_mut().insert(addr);

              #[cfg(feature = "signals")]
              {
                let mut req_meta: HashMap<String, String, BuildHasher> =
                  HashMap::with_hasher(BuildHasher::default());
                req_meta.insert("method".to_string(), method.clone());
                req_meta.insert("path".to_string(), path.clone());
                SignalArbiter::emit_app(Signal::with_metadata(ids::REQUEST_STARTED, req_meta))
                  .await;
              }

              let response = r.dispatch(req.map(TakoBody::new)).await;

              #[cfg(feature = "signals")]
              {
                let mut done_meta: HashMap<String, String, BuildHasher> =
                  HashMap::with_hasher(BuildHasher::default());
                done_meta.insert("method".to_string(), method);
                done_meta.insert("path".to_string(), path);
                done_meta.insert("status".to_string(), response.status().as_u16().to_string());
                SignalArbiter::emit_app(Signal::with_metadata(
                  ids::REQUEST_COMPLETED,
                  done_meta,
                ))
                .await;
              }

              Ok::<_, Infallible>(response)
            }
          });

          #[cfg(feature = "http2")]
          if proto.as_deref() == Some(b"h2") {
            let h2 = http2::Builder::new(TokioExecutor::new());

            if let Err(e) = h2.serve_connection(io, svc).await {
              tracing::error!("HTTP/2 error: {e}");
            }

            #[cfg(feature = "signals")]
            {
              let mut conn_close_meta: HashMap<String, String, BuildHasher> =
                HashMap::with_hasher(BuildHasher::default());
              conn_close_meta.insert("remote_addr".to_string(), addr.to_string());
              conn_close_meta.insert("tls".to_string(), "true".to_string());
              SignalArbiter::emit_app(Signal::with_metadata(
                ids::CONNECTION_CLOSED,
                conn_close_meta,
              ))
              .await;
            }

            if inflight.fetch_sub(1, Ordering::SeqCst) == 1 {
              drain_notify.notify_one();
            }
            return;
          }

          let mut h1 = http1::Builder::new();
          h1.keep_alive(true);

          if let Err(e) = h1.serve_connection(io, svc).with_upgrades().await {
            tracing::error!("HTTP/1.1 error: {e}");
          }

          #[cfg(feature = "signals")]
          {
            let mut conn_close_meta: HashMap<String, String, BuildHasher> =
              HashMap::with_hasher(BuildHasher::default());
            conn_close_meta.insert("remote_addr".to_string(), addr.to_string());
            conn_close_meta.insert("tls".to_string(), "true".to_string());
            SignalArbiter::emit_app(Signal::with_metadata(
              ids::CONNECTION_CLOSED,
              conn_close_meta,
            ))
            .await;
          }

          if inflight.fetch_sub(1, Ordering::SeqCst) == 1 {
            drain_notify.notify_one();
          }
        })
        .detach();
      }
      Either::Right(_) => {
        tracing::info!("Shutdown signal received, draining TLS connections...");
        break;
      }
    }
  }

  // Drain in-flight connections
  if inflight.load(Ordering::SeqCst) > 0 {
    let drain_wait = drain_notify.notified();
    let sleep = compio::time::sleep(DEFAULT_DRAIN_TIMEOUT);
    let drain_wait = std::pin::pin!(drain_wait);
    let sleep = std::pin::pin!(sleep);
    match futures_util::future::select(drain_wait, sleep).await {
      Either::Left(_) => {}
      Either::Right(_) => {
        tracing::warn!(
          "Drain timeout ({:?}) exceeded, {} TLS connections still active",
          DEFAULT_DRAIN_TIMEOUT,
          inflight.load(Ordering::SeqCst)
        );
      }
    }
  }

  tracing::info!("TLS server shut down gracefully");
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
pub fn load_key(path: &str) -> anyhow::Result<PrivateKeyDer<'static>> {
  let mut rd = BufReader::new(
    File::open(path).map_err(|e| anyhow::anyhow!("failed to open key file '{}': {}", path, e))?,
  );
  pkcs8_private_keys(&mut rd)
    .next()
    .ok_or_else(|| anyhow::anyhow!("no private key found in '{}'", path))?
    .map(|k| k.into())
    .map_err(|e| anyhow::anyhow!("bad private key in '{}': {}", path, e))
}
