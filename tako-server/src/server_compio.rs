use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use compio::net::TcpListener;
use cyper_core::HyperStream;
use futures_util::future::Either;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use tokio::sync::Notify;

use tako_core::body::TakoBody;
use tako_core::conn_info::ConnInfo;
use tako_core::router::Router;
#[cfg(feature = "signals")]
use tako_core::signals::transport as signal_tx;
use tako_core::types::BoxError;

use crate::ServerConfig;

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
  tako_core::tracing::init_tracing();

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
  let _max_connections = config.max_connections;

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
        let io = HyperStream::new(stream);
        let router = router.clone();
        let inflight = inflight.clone();
        let drain_notify = drain_notify.clone();

        inflight.fetch_add(1, Ordering::SeqCst);

        compio::runtime::spawn(async move {
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

          inflight.fetch_sub(1, Ordering::SeqCst);
          // Wake every drainer waiter — notify_one() races against waiters
          // registered between the load and the await on the coordinator side.
          drain_notify.notify_waiters();
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
