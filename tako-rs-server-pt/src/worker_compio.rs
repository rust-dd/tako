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

/// RAII counter-decrementer used by the compio worker to track in-flight
/// connections.
///
/// `Drop` always runs — normal completion, panic unwind, runtime shutdown —
/// so the inflight count cannot leak. Mirrors the `ConnectionGuard` pattern
/// in `tako-server`'s `server_compio.rs`. Without this the compio per-thread
/// worker had no way to wait for in-flight work at shutdown: it spawned and
/// detached connection tasks, and `cfg.drain_timeout` was silently ignored
/// — every active request was abort-killed the moment `block_on` returned.
#[cfg(feature = "compio")]
struct PtConnGuard {
  inflight: std::sync::Arc<std::sync::atomic::AtomicUsize>,
  drain_notify: std::sync::Arc<tokio::sync::Notify>,
}

#[cfg(feature = "compio")]
impl PtConnGuard {
  fn new(
    inflight: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    drain_notify: std::sync::Arc<tokio::sync::Notify>,
  ) -> Self {
    inflight.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    Self {
      inflight,
      drain_notify,
    }
  }
}

#[cfg(feature = "compio")]
impl Drop for PtConnGuard {
  fn drop(&mut self) {
    self
      .inflight
      .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    self.drain_notify.notify_waiters();
  }
}

#[cfg(feature = "compio")]
#[cfg_attr(not(feature = "affinity"), allow(unused_variables))]
pub(crate) fn worker_main_compio(
  worker_id: usize,
  addr: SocketAddr,
  router: &'static Router,
  cfg: PerThreadConfig,
  shutdown: PerThreadShutdown,
) {
  use std::sync::Arc;
  use std::sync::atomic::AtomicUsize;
  use std::sync::atomic::Ordering;

  use cyper_core::HyperStream;
  use tokio::sync::Notify;

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
      // Unblock the parent's `wait_for_bind_outcome` on runtime-build
      // failure too (worker never reaches its bind step otherwise).
      shutdown.report_bind_failure(io::Error::other(format!(
        "worker {worker_id}: failed to build compio runtime: {e}"
      )));
      return;
    }
  };

  rt.block_on(async move {
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
    let inflight = Arc::new(AtomicUsize::new(0));
    let drain_notify = Arc::new(Notify::new());

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
          compio::time::sleep(delay).await;
          backoff = std::cmp::min(backoff * 2, Duration::from_secs(1));
          continue;
        }
        futures_util::future::Either::Right(_) => {
          tracing::info!("worker {worker_id}: shutdown signalled, draining");
          break;
        }
      };
      // Match the tokio variant: disable Nagle so HTTP/1 small writes don't
      // pay a 40ms RTT penalty on the wire. Mirrors the tokio-pt path at the
      // top of this file.
      if let Err(e) = stream.set_nodelay(true) {
        tracing::debug!("worker {worker_id}: set_nodelay failed for {peer}: {e}");
      }
      let io = HyperStream::new(stream);
      // Build the guard before spawn so the count is incremented on the
      // current thread (lock-free atomic) instead of racing with the spawn.
      let guard = PtConnGuard::new(inflight.clone(), drain_notify.clone());

      compio::runtime::spawn(async move {
        // RAII: dropping `_guard` (on normal completion, panic, or task
        // cancellation) decrements `inflight` and wakes drain waiters.
        let _guard = guard;
        let svc = service_fn(move |mut req| async move {
          // Match the tokio variant: insert both the raw `SocketAddr`
          // (legacy lookup key) and the typed `ConnInfo` so extractors that
          // key off either type observe the same runtime regardless of
          // whether the build is `compio` or `tokio`. The compio path used
          // to insert only `peer`, breaking extractors that look up
          // `ConnInfo` (notably the IP-trust / forwarded-host helpers).
          req.extensions_mut().insert(peer);
          req.extensions_mut().insert(ConnInfo::tcp(peer));
          let resp = router
            .dispatch(req.map(tako_rs_core::body::TakoBody::new))
            .await;
          Ok::<_, Infallible>(resp)
        });

        let mut http = http1::Builder::new();
        http.keep_alive(true);
        if let Err(err) = http.serve_connection(io, svc).with_upgrades().await {
          if err.is_incomplete_message() {
            tracing::debug!("worker {worker_id}: client disconnected mid-message: {err}");
          } else {
            tracing::error!("worker {worker_id}: connection error: {err}");
          }
        }
      })
      .detach();
    }

    // Drain phase: wait for in-flight connections to finish, but only up to
    // `cfg.drain_timeout`. Mirrors the tokio worker (`join_all` + timeout)
    // and the standalone compio server's `inflight + Notify` loop. Without
    // this the compio worker silently aborted every active connection on
    // shutdown — `drain_timeout` was a no-op on the per-thread + compio
    // build.
    let drain_deadline = std::time::Instant::now() + cfg.drain_timeout;
    while inflight.load(Ordering::SeqCst) > 0 {
      let now = std::time::Instant::now();
      if now >= drain_deadline {
        tracing::warn!(
          worker_id,
          drain_timeout = ?cfg.drain_timeout,
          still_inflight = inflight.load(Ordering::SeqCst),
          "drain timeout exceeded; remaining connections will be aborted"
        );
        break;
      }
      let remaining = drain_deadline - now;
      let wait = drain_notify.notified();
      let sleep = compio::time::sleep(remaining);
      let wait = std::pin::pin!(wait);
      let sleep = std::pin::pin!(sleep);
      if let futures_util::future::Either::Right(_) =
        futures_util::future::select(wait, sleep).await
      {
        tracing::warn!(
          worker_id,
          drain_timeout = ?cfg.drain_timeout,
          still_inflight = inflight.load(Ordering::SeqCst),
          "drain timeout exceeded; remaining connections will be aborted"
        );
        break;
      }
    }
  });
}
