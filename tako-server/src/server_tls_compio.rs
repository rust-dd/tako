#![cfg(feature = "tls")]
#![cfg_attr(docsrs, doc(cfg(feature = "tls")))]

//! TLS-enabled HTTP server implementation for secure connections (compio runtime).
//!
//! # `send_wrapper` invariant — hard contract
//!
//! Hyper's HTTP/2 server builder requires `Send` on the response future and
//! the executor it hands work to. The compio runtime is **single-threaded
//! per core**: every future created by `compio::runtime::spawn` is `!Send`
//! and is polled exclusively on the runtime thread that produced it.
//!
//! Reconciling these two facts is the entire reason `send_wrapper` shows up
//! in this file:
//!
//! * [`ServiceSendWrapper`] wraps the per-connection hyper service and its
//!   response future in `SendWrapper`, satisfying hyper's bound at the type
//!   level.
//! * [`CompioH2Executor`] re-`spawn`s those `Send`-claimed futures back onto
//!   the same compio runtime thread.
//! * [`CompioH2Timer`] wraps `compio::time::sleep` similarly so HTTP/2
//!   keep-alive timers can be handed to hyper.
//!
//! **The soundness of this pattern depends on the wrapped values never
//! crossing a thread boundary at runtime.** That holds because:
//!
//! 1. The compio runtime is per-thread — futures are pinned to the thread
//!    that called `spawn`, and there is no cross-thread work-stealing.
//! 2. `SendWrapper<T>` panics on drop or deref from any thread other than
//!    the one that constructed it, so an accidental cross-thread move
//!    becomes a loud panic instead of UB.
//! 3. We never construct a `SendWrapper` outside of a compio runtime task,
//!    and we never hand the wrapper to a multi-threaded tokio runtime.
//!
//! The `Send` claim made by `SendWrapper<T>` is therefore **per-runtime, not
//! global**. Anyone moving these types out of the compio path (e.g. mixing
//! a tokio executor in front of `ServiceSendWrapper`) breaks the invariant.

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
#[cfg(feature = "http2")]
use send_wrapper::SendWrapper;
use tako_core::body::TakoBody;
use tako_core::conn_info::ConnInfo;
use tako_core::conn_info::TlsInfo;
use tako_core::router::Router;
#[cfg(feature = "signals")]
use tako_core::signals::transport as signal_tx;
use tako_core::types::BoxError;
use tokio::sync::Notify;

use crate::ServerConfig;

// HTTP/2 hardening + connection lifetimes are sourced from `ServerConfig`,
// whose `Default` mirrors the historical hardcoded values.

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

/// Starts a TLS-enabled HTTP server (compio) with graceful shutdown support.
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

/// Like [`serve_tls`] with caller-supplied [`ServerConfig`].
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

/// Like [`serve_tls_with_shutdown`] with caller-supplied [`ServerConfig`].
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

/// Like [`serve_tls`] with a caller-built `Arc<rustls::ServerConfig>` (compio).
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

  #[cfg(feature = "plugins")]
  router.setup_plugins_once();

  let addr_str = listener.local_addr()?.to_string();

  #[cfg(feature = "signals")]
  signal_tx::emit_server_started(&addr_str, "tcp", true).await;

  tracing::info!("Tako TLS listening on {}", addr_str);

  let inflight = Arc::new(AtomicUsize::new(0));
  let drain_notify = Arc::new(Notify::new());
  let drain_timeout = config.drain_timeout;
  let keep_alive = config.keep_alive;
  #[cfg(feature = "http2")]
  let h2_max_concurrent_streams = config.h2_max_concurrent_streams;
  #[cfg(feature = "http2")]
  let h2_max_header_list_size = config.h2_max_header_list_size;
  #[cfg(feature = "http2")]
  let h2_max_send_buf_size = config.h2_max_send_buf_size;
  #[cfg(feature = "http2")]
  let h2_max_pending_accept_reset_streams = config.h2_max_pending_accept_reset_streams;

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
              inflight.fetch_sub(1, Ordering::SeqCst);
              drain_notify.notify_waiters();
              return;
            }
          };

          #[cfg(feature = "signals")]
          signal_tx::emit_connection_opened(&addr.to_string(), true, None).await;

          let alpn_proto = tls_stream.negotiated_alpn().map(|p| p.into_owned());
          let is_h2 = matches!(alpn_proto.as_deref(), Some(b"h2"));
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

            if let Err(e) = h2.serve_connection(io, ServiceSendWrapper::new(svc)).await {
              tracing::error!("HTTP/2 error: {e}");
            }

            #[cfg(feature = "signals")]
            signal_tx::emit_connection_closed(&addr.to_string(), true, None).await;

            inflight.fetch_sub(1, Ordering::SeqCst);
            drain_notify.notify_waiters();
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

          inflight.fetch_sub(1, Ordering::SeqCst);
          drain_notify.notify_waiters();
        })
        .detach();
      }
      Either::Right(_) => {
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

/// Loads TLS certificates from a PEM-encoded file. Re-export of
/// [`tako_core::tls::load_certs`].
pub use tako_core::tls::load_certs;
/// Loads a private key from a PEM-encoded file. Accepts PKCS#8, PKCS#1 (RSA),
/// and SEC1 (EC) PEM blocks. Re-export of [`tako_core::tls::load_key`].
pub use tako_core::tls::load_key;

//
// compio is a single-threaded, thread-per-core runtime whose futures are `!Send`.
// hyper's HTTP/2 builder needs an executor to spawn stream handlers and checks
// `Send` at compile time.  Since all spawned futures run on the same thread,
// wrapping them with `SendWrapper` is safe and satisfies the compiler.

/// Wraps a hyper `Service` so its response future type is `Send` via `SendWrapper`.
///
/// This is safe because compio is single-threaded — futures never cross thread
/// boundaries. The `Send` bound is purely a compile-time requirement from hyper's
/// HTTP/2 executor trait, not an actual thread-safety need.
#[cfg(feature = "http2")]
struct ServiceSendWrapper<T>(SendWrapper<T>);

#[cfg(feature = "http2")]
impl<T> ServiceSendWrapper<T> {
  fn new(inner: T) -> Self {
    Self(SendWrapper::new(inner))
  }
}

#[cfg(feature = "http2")]
impl<R, T> hyper::service::Service<R> for ServiceSendWrapper<T>
where
  T: hyper::service::Service<R>,
{
  type Response = T::Response;
  type Error = T::Error;
  type Future = SendWrapper<T::Future>;

  fn call(&self, req: R) -> Self::Future {
    SendWrapper::new(self.0.call(req))
  }
}

/// A hyper executor for compio that accepts `!Send` futures.
///
/// Unlike `cyper_core::CompioExecutor` which requires `F: Send`, this executor
/// accepts any `F: 'static` — but we only use it with `SendWrapper`-wrapped
/// futures, so the `Send` bound is satisfied through the wrapper.
#[cfg(feature = "http2")]
#[derive(Debug, Clone)]
struct CompioH2Executor;

#[cfg(feature = "http2")]
impl<F: std::future::Future<Output = ()> + Send + 'static> hyper::rt::Executor<F>
  for CompioH2Executor
{
  fn execute(&self, fut: F) {
    compio::runtime::spawn(fut).detach();
  }
}

/// A hyper `Timer` implementation backed by `compio::time`.
///
/// Required for HTTP/2 keep-alive pings, stream timeouts, etc.
/// Wraps compio's `!Send` sleep futures in `SendWrapper` to satisfy hyper's bounds.
#[cfg(feature = "http2")]
#[derive(Debug, Clone)]
struct CompioH2Timer;

/// A sleep future that wraps a compio sleep, made `Send + Sync + Unpin` for hyper.
///
/// SAFETY: compio is a single-threaded runtime — the sleep future never crosses
/// thread boundaries, so `Send`/`Sync` are safe to implement unconditionally.
/// This is the same pattern used by `cyper-core::CompioTimer`.
#[cfg(feature = "http2")]
struct CompioSleep(std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>);

#[cfg(feature = "http2")]
impl std::future::Future for CompioSleep {
  type Output = ();

  fn poll(
    mut self: std::pin::Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<Self::Output> {
    self.0.as_mut().poll(cx)
  }
}

// SAFETY: compio is single-threaded — the sleep future never crosses threads.
#[cfg(feature = "http2")]
unsafe impl Send for CompioSleep {}
#[cfg(feature = "http2")]
unsafe impl Sync for CompioSleep {}

#[cfg(feature = "http2")]
impl Unpin for CompioSleep {}

#[cfg(feature = "http2")]
impl hyper::rt::Sleep for CompioSleep {}

#[cfg(feature = "http2")]
impl hyper::rt::Timer for CompioH2Timer {
  fn sleep(&self, duration: std::time::Duration) -> std::pin::Pin<Box<dyn hyper::rt::Sleep>> {
    Box::pin(CompioSleep(Box::pin(compio::time::sleep(duration))))
  }

  fn sleep_until(&self, deadline: std::time::Instant) -> std::pin::Pin<Box<dyn hyper::rt::Sleep>> {
    Box::pin(CompioSleep(Box::pin(compio::time::sleep_until(deadline))))
  }
}
