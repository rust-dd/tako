//! The compio TLS accept/serve loop: per-connection handshake, protocol
//! dispatch (HTTP/1.1 and HTTP/2), graceful shutdown, and connection draining.

use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use compio::net::TcpListener;
use compio::tls::TlsAcceptor;
use cyper_core::HyperStream;
use futures_util::future::Either;
use hyper::server::conn::http1;
#[cfg(feature = "http2")]
use hyper::server::conn::http2;
use hyper::service::service_fn;
use rustls::ServerConfig as RustlsServerConfig;
use tako_rs_core::body::TakoBody;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::conn_info::TlsInfo;
use tako_rs_core::router::Router;
#[cfg(feature = "signals")]
use tako_rs_core::signals::transport as signal_tx;
use tako_rs_core::types::BoxError;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::ServerConfig;
#[cfg(feature = "http2")]
use crate::server_tls_compio::executor::CompioH2Executor;
#[cfg(feature = "http2")]
use crate::server_tls_compio::executor::CompioH2Timer;
#[cfg(feature = "http2")]
use crate::server_tls_compio::executor::ServiceSendWrapper;

// HTTP/2 hardening + connection lifetimes are sourced from `ServerConfig`,
// whose `Default` mirrors the historical hardcoded values.

/// Variant of [`run`](super::run) that accepts a pre-built `Arc<rustls::ServerConfig>`.
pub async fn run_with_config(
  listener: TcpListener,
  router: Router,
  tls_config: Arc<RustlsServerConfig>,
  signal: Option<impl Future<Output = ()>>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  #[cfg(feature = "tako-tracing")]
  tako_rs_core::tracing::init_tracing();

  let acceptor = TlsAcceptor::from(tls_config);
  let router = Arc::new(router);

  #[cfg(feature = "plugins")]
  router.setup_plugins_once();

  let addr_str = listener.local_addr()?.to_string();

  #[cfg(feature = "signals")]
  signal_tx::emit_server_started(&addr_str, "tcp", true).await;

  tracing::info!("Tako TLS listening on {}", addr_str);

  let inflight = Arc::new(AtomicUsize::new(0));
  let drain_notify = Arc::new(Notify::new());
  let drain_timeout = config.drain_timeout;
  let tls_handshake_timeout = config.tls_handshake_timeout;
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
  // C14 (compio TLS): honor `max_connections` instead of silently ignoring it.
  let max_conn_semaphore = config
    .max_connections
    .map(|n| Arc::new(tokio::sync::Semaphore::new(n)));
  // C15 (compio TLS): per-loop accept backoff to survive transient
  // EMFILE / ConnectionAborted errors rather than exiting the server.
  let mut accept_backoff = config.accept_backoff;

  // SRV-11: shared shutdown signal across the accept loop AND every spawned
  // TLS-handshake task. Without this the per-connection task only saw the
  // handshake-deadline timer; an in-flight handshake could still hold a
  // `max_connections` permit for up to `tls_handshake_timeout` after the
  // operator pressed Ctrl+C, delaying graceful drain. The parent loop fires
  // `cancel.cancel()` as soon as `signal_fused` resolves so connections
  // already past `accept()` observe the same shutdown.
  let cancel = CancellationToken::new();
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
        let (stream, addr) = match result {
          Ok(v) => {
            accept_backoff.reset();
            v
          }
          Err(err) => {
            tracing::warn!("compio TLS accept failed: {err}; backing off");
            let d = accept_backoff.current_and_grow();
            // SRV-06: race backoff against shutdown so a 1s sleep cannot
            // delay graceful shutdown when the signal fires mid-backoff.
            let sleep = std::pin::pin!(compio::time::sleep(d));
            match futures_util::future::select(sleep, signal_fused.as_mut()).await {
              Either::Left(((), _)) => continue,
              Either::Right(_) => {
                cancel.cancel();
                break;
              }
            }
          }
        };

        // C14: hold the permit across the TLS handshake + connection lifetime.
        // Race against shutdown so a saturated cap can't deadlock the drain.
        let permit = if let Some(sem) = max_conn_semaphore.as_ref() {
          let acquire = std::pin::pin!(sem.clone().acquire_owned());
          match futures_util::future::select(acquire, signal_fused.as_mut()).await {
            Either::Left((Ok(p), _)) => Some(p),
            Either::Left((Err(_), _)) => continue,
            Either::Right(_) => {
              cancel.cancel();
              break;
            }
          }
        } else {
          None
        };

        let acceptor = acceptor.clone();
        let router = router.clone();
        let guard =
          crate::server_compio::ConnectionGuard::new(inflight.clone(), drain_notify.clone());
        let conn_cancel = cancel.clone();

        compio::runtime::spawn(async move {
          let _permit = permit;
          // RAII guard. Dropping `_guard` decrements `inflight` and wakes
          // drain waiters on any exit path — error, timeout, panic, or
          // success — so we no longer need manual `fetch_sub` calls.
          let _guard = guard;
          // Bound the TLS handshake against slowloris-style holds on the
          // `max_connections` permit. compio has no `tokio::time::timeout`
          // adapter, so race the accept future against an explicit
          // `compio::time::sleep` deadline AND against the shared shutdown
          // token so an in-flight handshake doesn't hold the permit past
          // graceful-shutdown initiation.
          let handshake_deadline = std::pin::pin!(compio::time::sleep(tls_handshake_timeout));
          let shutdown_wait = std::pin::pin!(conn_cancel.cancelled());
          let deadline_or_shutdown = std::pin::pin!(futures_util::future::select(
            handshake_deadline,
            shutdown_wait
          ));
          let accept_fut = std::pin::pin!(acceptor.accept(stream));
          let tls_stream =
            match futures_util::future::select(accept_fut, deadline_or_shutdown).await {
              Either::Left((Ok(s), _)) => s,
              Either::Left((Err(e), _)) => {
                tracing::error!("TLS error: {e}");
                return;
              }
              Either::Right((Either::Left(_), _)) => {
                tracing::warn!("TLS handshake timeout after {tls_handshake_timeout:?} from {addr}");
                return;
              }
              Either::Right((Either::Right(_), _)) => {
                tracing::debug!("TLS handshake aborted by shutdown from {addr}");
                return;
              }
            };

          #[cfg(feature = "signals")]
          signal_tx::emit_connection_opened(&addr.to_string(), true, None).await;

          let alpn_proto = tls_stream
            .negotiated_alpn()
            .map(std::borrow::Cow::into_owned);
          let is_h2 = matches!(alpn_proto.as_deref(), Some(b"h2"));
          // SNI and the negotiated TLS protocol version are NOT exposed by
          // `compio-tls::TlsStream` as of 0.9.1 — its public surface is only
          // `negotiated_alpn()`. The inner `futures_rustls::TlsStream` does
          // expose `server_name()` and `protocol_version()` accessors, but
          // `compio-tls`'s `TlsStreamInner` is private (no `get_ref()`), so
          // they are unreachable from this crate.
          //
          // Until upstream lands a `get_ref()`-style accessor (or surfaces
          // `server_name()` / `protocol_version()` directly), SNI and version
          // stay `None` on the compio path — workloads that depend on
          // SNI-based vhost routing, mTLS peer-cert hooks, or TLS-version
          // observability must use the tokio-rustls path (`server_tls.rs`),
          // which populates both. Tracking: tako audit §9.3 SRV-04.
          let conn_info = if is_h2 {
            ConnInfo::h2_tls(
              addr,
              TlsInfo {
                alpn: alpn_proto.clone(),
                sni: None,
                version: None,
              },
            )
          } else {
            ConnInfo::h1_tls(
              addr,
              TlsInfo {
                alpn: alpn_proto.clone(),
                sni: None,
                version: None,
              },
            )
          };

          #[cfg(feature = "http2")]
          let proto = alpn_proto;

          let io = HyperStream::new(tls_stream);
          // Per-request signals fire from inside Router::dispatch.
          let svc = service_fn(move |mut req| {
            let r = router.clone();
            let conn_info = conn_info.clone();
            async move {
              req.extensions_mut().insert(addr);
              req.extensions_mut().insert(conn_info);
              let response = r.dispatch(req.map(TakoBody::new)).await;
              Ok::<_, Infallible>(response)
            }
          });

          #[cfg(feature = "http2")]
          if proto.as_deref() == Some(b"h2") {
            let mut h2 = http2::Builder::new(CompioH2Executor);
            h2.timer(CompioH2Timer)
              .max_concurrent_streams(h2_max_concurrent_streams)
              .max_header_list_size(h2_max_header_list_size)
              .max_send_buf_size(h2_max_send_buf_size)
              .max_pending_accept_reset_streams(h2_max_pending_accept_reset_streams);
            if let Some(interval) = h2_keep_alive_interval {
              h2.keep_alive_interval(Some(interval));
            }

            if let Err(e) = h2.serve_connection(io, ServiceSendWrapper::new(svc)).await {
              tracing::error!("HTTP/2 error: {e}");
            }

            #[cfg(feature = "signals")]
            signal_tx::emit_connection_closed(&addr.to_string(), true, None).await;
            // `_guard` drops here on the H2 success/error path.
            return;
          }

          let mut h1 = http1::Builder::new();
          h1.keep_alive(keep_alive);

          if let Err(e) = h1.serve_connection(io, svc).with_upgrades().await {
            if e.is_incomplete_message() {
              tracing::debug!("TLS HTTP/1.1 client disconnected mid-message: {e}");
            } else {
              tracing::error!("HTTP/1.1 error: {e}");
            }
          }

          #[cfg(feature = "signals")]
          signal_tx::emit_connection_closed(&addr.to_string(), true, None).await;
          // `_guard` drops here on the H1 path, decrementing `inflight`.
        })
        .detach();
      }
      Either::Right(_) => {
        cancel.cancel();
        tracing::info!("Shutdown signal received, draining TLS connections...");
        break;
      }
    }
  }

  // Drain in-flight connections — re-check the inflight counter on every
  // notification, bounded by the overall drain deadline. Defends against the
  // race where a connection finishes between the load and the await.
  let drain_deadline = std::time::Instant::now() + drain_timeout;
  while inflight.load(Ordering::SeqCst) > 0 {
    let now = std::time::Instant::now();
    if now >= drain_deadline {
      tracing::warn!(
        "Drain timeout ({:?}) exceeded, {} TLS connections still active",
        drain_timeout,
        inflight.load(Ordering::SeqCst)
      );
      break;
    }
    let remaining = drain_deadline - now;
    let drain_wait = drain_notify.notified();
    let sleep = compio::time::sleep(remaining);
    let drain_wait = std::pin::pin!(drain_wait);
    let sleep = std::pin::pin!(sleep);
    if let Either::Right(_) = futures_util::future::select(drain_wait, sleep).await {
      tracing::warn!(
        "Drain timeout ({:?}) exceeded, {} TLS connections still active",
        drain_timeout,
        inflight.load(Ordering::SeqCst)
      );
      break;
    }
  }

  tracing::info!("TLS server shut down gracefully");
  Ok(())
}
