use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::router::Router;

use crate::config::PerThreadConfig;
use crate::listener::bind_reuseport_compio;
use crate::shutdown::PerThreadShutdown;

#[cfg(feature = "compio")]
fn compio_accept_backoff() -> Duration {
  Duration::from_millis(5)
}

#[cfg(feature = "compio")]
#[cfg_attr(not(feature = "affinity"), allow(unused_variables))]
pub(crate) fn worker_main_compio(
  worker_id: usize,
  addr: SocketAddr,
  router: std::sync::Arc<Router>,
  cfg: PerThreadConfig,
  shutdown: PerThreadShutdown,
) {
  use std::sync::Arc;

  use cyper_core::HyperStream;
  use futures_util::FutureExt;
  use futures_util::StreamExt;
  use tako_rs_core::server_support::drive_connection;

  #[cfg(feature = "affinity")]
  if cfg.pin_to_core {
    if let Some(ids) = core_affinity::get_core_ids() {
      if let Some(id) = ids.get(worker_id) {
        if !core_affinity::set_for_current(*id) {
          tracing::warn!(
            worker_id,
            "pin_to_core: core_affinity::set_for_current returned false; running without affinity"
          );
        }
      } else {
        tracing::warn!(
          worker_id,
          available_cores = ids.len(),
          "pin_to_core: worker_id exceeds available cores; running without affinity"
        );
      }
    } else {
      tracing::warn!(
        worker_id,
        "pin_to_core: core_affinity::get_core_ids() returned None; running without affinity"
      );
    }
  }

  let rt = match compio::runtime::RuntimeBuilder::new().build() {
    Ok(rt) => rt,
    Err(e) => {
      tracing::error!("worker {worker_id}: failed to build compio runtime: {e}");

      shutdown.report_bind_failure(io::Error::other(format!(
        "worker {worker_id}: failed to build compio runtime: {e}"
      )));
      return;
    }
  };

  rt.block_on(async move {
    #[cfg(feature = "plugins")]
    if let Err(error) = router.setup_plugins_once() {
      shutdown.report_bind_failure(io::Error::other(error.clone()));
      return;
    }
    let semaphore = cfg
      .max_connections
      .map(|n| Arc::new(tokio::sync::Semaphore::new(n)));
    let listener = match bind_reuseport_compio(addr, cfg.backlog) {
      Ok(l) => {
        shutdown.report_bind_success();
        l
      }
      Err(e) => {
        tracing::error!("worker {worker_id}: bind failed: {e}");
        shutdown.report_bind_failure(e);
        return;
      }
    };
    tracing::debug!("tako-pt-compio worker {worker_id} listening on {addr}");

    let cancel = shutdown.inner.clone();
    let mut backoff = compio_accept_backoff();
    let mut connections = futures_util::stream::FuturesUnordered::new();

    loop {
      let accept_fut = listener.accept();
      let cancel_fut = cancel.cancelled();
      tokio::pin!(accept_fut, cancel_fut);
      let accept = futures_util::future::select(accept_fut, cancel_fut).await;
      let (stream, peer) = match accept {
        futures_util::future::Either::Left((Ok(v), _)) => {
          backoff = compio_accept_backoff();
          v
        }
        futures_util::future::Either::Left((Err(e), _)) => {
          let delay = backoff;
          tracing::warn!("worker {worker_id}: accept failed: {e}; backing off {delay:?}");
          let sleep = std::pin::pin!(compio::time::sleep(delay));
          let stopped = std::pin::pin!(cancel.cancelled());
          if let futures_util::future::Either::Right(_) =
            futures_util::future::select(sleep, stopped).await
          {
            break;
          }
          backoff = std::cmp::min(backoff * 2, Duration::from_secs(1));
          continue;
        }
        futures_util::future::Either::Right(_) => {
          tracing::info!("worker {worker_id}: shutdown signalled, draining");
          break;
        }
      };

      if let Err(e) = stream.set_nodelay(true) {
        tracing::debug!("worker {worker_id}: set_nodelay failed for {peer}: {e}");
      }
      let permit = if let Some(semaphore) = &semaphore {
        let acquire = std::pin::pin!(semaphore.clone().acquire_owned());
        let stopped = std::pin::pin!(cancel.cancelled());
        match futures_util::future::select(acquire, stopped).await {
          futures_util::future::Either::Left((result, _)) => result.ok(),
          futures_util::future::Either::Right(_) => break,
        }
      } else {
        None
      };
      let io = HyperStream::new_plain(stream);
      let router = router.clone();
      let conn_cancel = cancel.clone();

      connections.push(compio::runtime::spawn(async move {
        let _permit = permit;
        let svc = service_fn(move |mut req| {
          let router = router.clone();
          async move {
            req.extensions_mut().insert(peer);
            req.extensions_mut().insert(ConnInfo::tcp(peer));
            let resp = router
              .dispatch(req.map(tako_rs_core::body::TakoBody::new))
              .await;
            Ok::<_, Infallible>(resp)
          }
        });

        let mut http = http1::Builder::new();
        http.keep_alive(true);
        http
          .timer(cyper_core::CompioTimer)
          .header_read_timeout(cfg.header_read_timeout);
        if let Err(err) = drive_connection(
          http.serve_connection(io, svc).with_upgrades(),
          conn_cancel.cancelled(),
          hyper::server::conn::http1::UpgradeableConnection::graceful_shutdown,
        )
        .await
        {
          if err.is_incomplete_message() {
            tracing::debug!("worker {worker_id}: client disconnected mid-message: {err}");
          } else {
            tracing::error!("worker {worker_id}: connection error: {err}");
          }
        }
      }));
      while connections.next().now_or_never().flatten().is_some() {}
    }

    if compio::time::timeout(cfg.drain_timeout, async {
      while connections.next().await.is_some() {}
    })
    .await
    .is_err()
    {
      for connection in connections {
        connection.cancel().await;
      }
    }
  });
}
