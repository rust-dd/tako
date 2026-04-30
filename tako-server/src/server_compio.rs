use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use compio::net::TcpListener;
use cyper_core::HyperStream;
use futures_util::future::Either;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use tokio::sync::Notify;

use tako_core::body::TakoBody;
use tako_core::router::Router;
#[cfg(feature = "signals")]
use tako_core::signals::Signal;
#[cfg(feature = "signals")]
use tako_core::signals::SignalArbiter;
#[cfg(feature = "signals")]
use tako_core::signals::ids;
use tako_core::types::BoxError;

/// Default drain timeout for graceful shutdown (30 seconds).
const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn serve(listener: TcpListener, router: Router) {
  if let Err(e) = run(listener, router, None::<std::future::Pending<()>>).await {
    tracing::error!("Server error: {e}");
  }
}

/// Starts the Tako HTTP server (compio) with graceful shutdown support.
pub async fn serve_with_shutdown(
  listener: TcpListener,
  router: Router,
  signal: impl Future<Output = ()>,
) {
  if let Err(e) = run(listener, router, Some(signal)).await {
    tracing::error!("Server error: {e}");
  }
}

async fn run(
  listener: TcpListener,
  router: Router,
  signal: Option<impl Future<Output = ()>>,
) -> Result<(), BoxError> {
  #[cfg(feature = "tako-tracing")]
  tako_core::tracing::init_tracing();

  let router = Arc::new(router);
  #[cfg(feature = "plugins")]
  router.setup_plugins_once();

  let addr_str = listener.local_addr()?.to_string();

  #[cfg(feature = "signals")]
  {
    SignalArbiter::emit_app(
      Signal::with_capacity(ids::SERVER_STARTED, 3)
        .meta("addr", addr_str.clone())
        .meta("transport", "tcp")
        .meta("tls", "false"),
    )
    .await;
  }

  tracing::debug!("Tako listening on {}", addr_str);

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
        let io = HyperStream::new(stream);
        let router = router.clone();
        let inflight = inflight.clone();
        let drain_notify = drain_notify.clone();

        inflight.fetch_add(1, Ordering::SeqCst);

        compio::runtime::spawn(async move {
          #[cfg(feature = "signals")]
          {
            SignalArbiter::emit_app(
              Signal::with_capacity(ids::CONNECTION_OPENED, 1)
                .meta("remote_addr", addr.to_string()),
            )
            .await;
          }

          let svc = service_fn(move |mut req| {
            let router = router.clone();
            async move {
              #[cfg(feature = "signals")]
              let path = req.uri().path().to_string();
              #[cfg(feature = "signals")]
              let method = req.method().to_string();

              req.extensions_mut().insert(addr);

              #[cfg(feature = "signals")]
              {
                SignalArbiter::emit_app(
                  Signal::with_capacity(ids::REQUEST_STARTED, 2)
                    .meta("method", method.clone())
                    .meta("path", path.clone()),
                )
                .await;
              }

              let response = router.dispatch(req.map(TakoBody::new)).await;

              #[cfg(feature = "signals")]
              {
                SignalArbiter::emit_app(
                  Signal::with_capacity(ids::REQUEST_COMPLETED, 3)
                    .meta("method", method)
                    .meta("path", path)
                    .meta("status", response.status().as_u16().to_string()),
                )
                .await;
              }

              Ok::<_, Infallible>(response)
            }
          });

          let mut http = http1::Builder::new();
          http.keep_alive(true);
          let conn = http.serve_connection(io, svc).with_upgrades();

          if let Err(err) = conn.await {
            if err.is_incomplete_message() {
              tracing::debug!("client disconnected mid-message: {err}");
            } else {
              tracing::error!("Error serving connection: {err}");
            }
          }

          #[cfg(feature = "signals")]
          {
            SignalArbiter::emit_app(
              Signal::with_capacity(ids::CONNECTION_CLOSED, 1)
                .meta("remote_addr", addr.to_string()),
            )
            .await;
          }

          if inflight.fetch_sub(1, Ordering::SeqCst) == 1 {
            drain_notify.notify_one();
          }
        })
        .detach();
      }
      Either::Right(_) => {
        tracing::info!("Shutdown signal received, draining connections...");
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
          "Drain timeout ({:?}) exceeded, {} connections still active",
          DEFAULT_DRAIN_TIMEOUT,
          inflight.load(Ordering::SeqCst)
        );
      }
    }
  }

  tracing::info!("Server shut down gracefully");
  Ok(())
}
