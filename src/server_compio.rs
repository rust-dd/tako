#[cfg(feature = "signals")]
use std::collections::HashMap;
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
  crate::tracing::init_tracing();

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
    server_meta.insert("tls".to_string(), "false".to_string());
    SignalArbiter::emit_app(Signal::with_metadata(ids::SERVER_STARTED, server_meta)).await;
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
            let mut conn_open_meta: HashMap<String, String, BuildHasher> =
              HashMap::with_hasher(BuildHasher::default());
            conn_open_meta.insert("remote_addr".to_string(), addr.to_string());
            SignalArbiter::emit_app(Signal::with_metadata(
              ids::CONNECTION_OPENED,
              conn_open_meta,
            ))
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
                let mut req_meta: HashMap<String, String, BuildHasher> =
                  HashMap::with_hasher(BuildHasher::default());
                req_meta.insert("method".to_string(), method.clone());
                req_meta.insert("path".to_string(), path.clone());
                SignalArbiter::emit_app(Signal::with_metadata(ids::REQUEST_STARTED, req_meta))
                  .await;
              }

              let response = router.dispatch(req.map(TakoBody::new)).await;

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

          let mut http = http1::Builder::new();
          http.keep_alive(true);
          let conn = http.serve_connection(io, svc).with_upgrades();

          if let Err(err) = conn.await {
            tracing::error!("Error serving connection: {err}");
          }

          #[cfg(feature = "signals")]
          {
            let mut conn_close_meta: HashMap<String, String, BuildHasher> =
              HashMap::with_hasher(BuildHasher::default());
            conn_close_meta.insert("remote_addr".to_string(), addr.to_string());
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
