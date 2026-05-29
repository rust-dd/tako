use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use compio::net::TcpListener;
use cyper_core::HyperStream;
use futures_util::future::Either;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use tako_rs_core::body::TakoBody;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::router::Router;
#[cfg(feature = "signals")]
use tako_rs_core::signals::transport as signal_tx;
use tako_rs_core::types::BoxError;
use tokio::sync::Notify;

use crate::ServerConfig;

/// RAII guard that increments `inflight` on construction and decrements it on
/// drop, then wakes drain waiters. Captured into the spawned connection task
/// so the counter stays consistent under panic, spawn failure, or any control
/// flow that does not reach an explicit `fetch_sub`. Replaces the previous
/// "`fetch_add` before spawn + manual `fetch_sub` at the end" pattern which leaked
/// counts on any panic between the two operations.
pub(crate) struct ConnectionGuard {
  inflight: Arc<AtomicUsize>,
  drain_notify: Arc<Notify>,
}

impl ConnectionGuard {
  pub(crate) fn new(inflight: Arc<AtomicUsize>, drain_notify: Arc<Notify>) -> Self {
    inflight.fetch_add(1, Ordering::SeqCst);
    Self {
      inflight,
      drain_notify,
    }
  }
}

impl Drop for ConnectionGuard {
  fn drop(&mut self) {
    self.inflight.fetch_sub(1, Ordering::SeqCst);
    self.drain_notify.notify_waiters();
  }
}

pub async fn serve(listener: TcpListener, router: Router) {
  if let Err(e) = run(
    listener,
    router,
    None::<std::future::Pending<()>>,
    ServerConfig::default(),
  )
  .await
  {
    tracing::error!("Server error: {e}");
  }
}

/// Starts the Tako HTTP server (compio) with graceful shutdown support.
pub async fn serve_with_shutdown(
  listener: TcpListener,
  router: Router,
  signal: impl Future<Output = ()>,
) {
  if let Err(e) = run(listener, router, Some(signal), ServerConfig::default()).await {
    tracing::error!("Server error: {e}");
  }
}

/// Like [`serve`] with caller-supplied [`ServerConfig`].
pub async fn serve_with_config(listener: TcpListener, router: Router, config: ServerConfig) {
  if let Err(e) = run(listener, router, None::<std::future::Pending<()>>, config).await {
    tracing::error!("Server error: {e}");
  }
}

/// Like [`serve_with_shutdown`] with caller-supplied [`ServerConfig`].
pub async fn serve_with_shutdown_and_config(
  listener: TcpListener,
  router: Router,
  signal: impl Future<Output = ()>,
  config: ServerConfig,
) {
  if let Err(e) = run(listener, router, Some(signal), config).await {
    tracing::error!("Server error: {e}");
  }
}

async fn run(
  listener: TcpListener,
  router: Router,
  signal: Option<impl Future<Output = ()>>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  #[cfg(feature = "tako-tracing")]
  tako_rs_core::tracing::init_tracing();

  let router = Arc::new(router);
  #[cfg(feature = "plugins")]
  router.setup_plugins_once();

  let addr_str = listener.local_addr()?.to_string();

  #[cfg(feature = "signals")]
  signal_tx::emit_server_started(&addr_str, "tcp", false).await;

  tracing::debug!("Tako listening on {}", addr_str);

  let inflight = Arc::new(AtomicUsize::new(0));
  let drain_notify = Arc::new(Notify::new());
  let drain_timeout = config.drain_timeout;
  let keep_alive = config.keep_alive;
  // C14: honor `max_connections` on the compio path. `tokio::sync::Semaphore`
  // is runtime-agnostic for `acquire_owned` (no tokio timer/IO required).
  let max_conn_semaphore = config
    .max_connections
    .map(|n| Arc::new(tokio::sync::Semaphore::new(n)));
  // C15: per-loop accept backoff for transient errors (EMFILE / ConnectionAborted).
  let mut accept_backoff = config.accept_backoff;

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
            // C15: don't kill the server on a single transient accept error.
            tracing::warn!("compio accept failed: {err}; backing off");
            let d = accept_backoff.current_and_grow();
            // SRV-06: race the backoff against the shutdown signal so a
            // 1s sleep cannot delay graceful shutdown by up to 1s if the
            // signal fires mid-backoff. `select`'s `Right` arm means the
            // signal won; break the loop so the drain path runs.
            let sleep = std::pin::pin!(compio::time::sleep(d));
            match futures_util::future::select(sleep, signal_fused.as_mut()).await {
              Either::Left(((), _)) => continue,
              Either::Right(_) => break,
            }
          }
        };

        // C14: park here until a permit is available, racing the wait
        // against the shutdown signal so a saturated cap can't deadlock
        // graceful shutdown.
        let permit = if let Some(sem) = max_conn_semaphore.as_ref() {
          let acquire = std::pin::pin!(sem.clone().acquire_owned());
          match futures_util::future::select(acquire, signal_fused.as_mut()).await {
            Either::Left((Ok(p), _)) => Some(p),
            Either::Left((Err(_), _)) => continue,
            Either::Right(_) => break,
          }
        } else {
          None
        };

        let io = HyperStream::new(stream);
        let router = router.clone();
        let guard = ConnectionGuard::new(inflight.clone(), drain_notify.clone());

        compio::runtime::spawn(async move {
          let _permit = permit;
          // RAII: dropping `_guard` (on normal completion, panic, or task
          // cancellation) decrements `inflight` and wakes drain waiters.
          let _guard = guard;
          #[cfg(feature = "signals")]
          signal_tx::emit_connection_opened(&addr.to_string(), false, None).await;

          let svc = service_fn(move |mut req| {
            let router = router.clone();
            async move {
              req.extensions_mut().insert(addr);
              req.extensions_mut().insert(ConnInfo::tcp(addr));
              let response = router.dispatch(req.map(TakoBody::new)).await;
              Ok::<_, Infallible>(response)
            }
          });

          let mut http = http1::Builder::new();
          http.keep_alive(keep_alive);
          let conn = http.serve_connection(io, svc).with_upgrades();

          if let Err(err) = conn.await {
            if err.is_incomplete_message() {
              tracing::debug!("client disconnected mid-message: {err}");
            } else {
              tracing::error!("Error serving connection: {err}");
            }
          }

          #[cfg(feature = "signals")]
          signal_tx::emit_connection_closed(&addr.to_string(), false, None).await;
        })
        .detach();
      }
      Either::Right(_) => {
        tracing::info!("Shutdown signal received, draining connections...");
        break;
      }
    }
  }

  // Drain in-flight connections — re-check inflight after every notification
  // and bail when the overall deadline elapses, so a connection that closes
  // between the load and the await still satisfies the drain.
  let drain_deadline = std::time::Instant::now() + drain_timeout;
  while inflight.load(Ordering::SeqCst) > 0 {
    let now = std::time::Instant::now();
    if now >= drain_deadline {
      tracing::warn!(
        "Drain timeout ({:?}) exceeded, {} connections still active",
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
        "Drain timeout ({:?}) exceeded, {} connections still active",
        drain_timeout,
        inflight.load(Ordering::SeqCst)
      );
      break;
    }
  }

  tracing::info!("Server shut down gracefully");
  Ok(())
}
