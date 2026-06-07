use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use tako_rs_core::body::TakoBody;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::router::Router;
use tokio::runtime::Builder;
use tokio::task::LocalSet;

use crate::config::PerThreadConfig;
use crate::listener::bind_reuseport;
use crate::shutdown::PerThreadShutdown;

// Without the `affinity` feature, `worker_id` and `cfg.pin_to_core` aren't
// read past this point; mark the function tolerant of those unused names so
// we don't need the awkward `let _ = (worker_id, &cfg.pin_to_core);` trick
// that previously sat inside the function body for the sole purpose of
// silencing the warning.
#[cfg_attr(not(feature = "affinity"), allow(unused_variables))]
pub(crate) fn worker_main(
  worker_id: usize,
  addr: SocketAddr,
  router: &'static Router,
  cfg: PerThreadConfig,
  shutdown: PerThreadShutdown,
) {
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

  let rt = match Builder::new_current_thread().enable_all().build() {
    Ok(rt) => rt,
    Err(e) => {
      tracing::error!("worker {worker_id}: failed to build runtime: {e}");
      // Treat runtime-build failure as a bind failure so the parent is
      // unblocked from `wait_for_bind_outcome` instead of waiting on
      // Ctrl+C for a worker that never reached its bind step.
      shutdown.report_bind_failure(io::Error::other(format!(
        "worker {worker_id}: failed to build runtime: {e}"
      )));
      return;
    }
  };

  let local = LocalSet::new();
  local.block_on(&rt, async move {
    let listener = match bind_reuseport(addr, cfg.backlog) {
      Ok(l) => {
        // Report success so `serve_per_thread` can stop blocking on
        // `wait_for_bind_outcome` and proceed to the Ctrl+C wait.
        shutdown.report_bind_success();
        l
      }
      Err(e) => {
        tracing::error!("worker {worker_id}: bind failed: {e}");
        shutdown.report_bind_failure(e);
        return;
      }
    };
    tracing::debug!("tako-pt worker {worker_id} listening on {addr}");

    let shutdown_fut = shutdown.notified();
    tokio::pin!(shutdown_fut);

    // SRV-07: track per-connection tasks in a `JoinSet` instead of a `Vec`
    // so completed connections can be reaped lazily. The previous `Vec`
    // grew unboundedly across a worker's lifetime (a million-connection
    // day = a million `JoinHandle`s held forever — soft leak proportional
    // to total connections handled, even though the underlying tasks were
    // long done).
    let mut connection_handles: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();

    loop {
      tokio::select! {
        accept = listener.accept() => {
          let (stream, peer) = match accept {
            Ok(v) => v,
            Err(e) => {
              tracing::warn!("worker {worker_id}: accept failed: {e}");
              continue;
            }
          };
          // `set_nodelay` only fails for already-closed sockets (peer hung
          // up between accept and here) or on platforms without TCP_NODELAY
          // — either way the connection still works, but log at debug so an
          // operator can investigate persistent failures.
          if let Err(e) = stream.set_nodelay(true) {
            tracing::debug!("worker {worker_id}: set_nodelay failed for {peer}: {e}");
          }
          let io = hyper_util::rt::TokioIo::new(stream);

          connection_handles.spawn_local(async move {
            let svc = service_fn(move |mut req| async move {
              req.extensions_mut().insert(peer);
              req.extensions_mut().insert(ConnInfo::tcp(peer));
              let resp = router.dispatch(req.map(TakoBody::incoming)).await;
              Ok::<_, Infallible>(resp)
            });

            let mut http = http1::Builder::new();
            http.keep_alive(true);
            http.pipeline_flush(true);
            if let Err(err) = http.serve_connection(io, svc).with_upgrades().await {
              if err.is_incomplete_message() {
                tracing::debug!("worker {worker_id}: client disconnected mid-message: {err}");
              } else {
                tracing::error!("worker {worker_id}: connection error: {err}");
              }
            }
          });

          // Reap finished connection tasks opportunistically so the JoinSet
          // does not retain `AbortHandle`s for already-completed tasks. Each
          // `try_join_next` is non-blocking; the loop drains all currently
          // finished entries.
          while connection_handles.try_join_next().is_some() {}
        }
        () = &mut shutdown_fut => {
          tracing::info!("worker {worker_id}: shutdown signalled, draining");
          break;
        }
      }
    }
    // Real graceful drain: wait on every in-flight connection task up to
    // `drain_timeout`. `join_next()` yields one task at a time as it
    // finishes; the timeout wraps the whole drain so a single hung
    // connection cannot stall shutdown forever.
    let drain = tokio::time::timeout(cfg.drain_timeout, async {
      while connection_handles.join_next().await.is_some() {}
    });
    let _ = drain.await;
  });
}
