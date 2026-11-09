use std::io;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use hyper::server::conn::http1;
use tako_rs_core::router::Router;
use tako_rs_core::server_support::ConnCtx;
use tako_rs_core::server_support::ConnDriver;
use tako_rs_core::server_support::Http1Io;
use tako_rs_core::server_support::Leased;
use tako_rs_core::server_support::ShutdownSignal;
use tako_rs_core::server_support::UpgradeSupervisor;
use tako_rs_core::server_support::tcp_service;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::runtime::Builder;
use tokio::sync::mpsc;
use tokio::task::LocalSet;

use crate::PerThreadError;
use crate::PerThreadErrorKind;
use crate::balance::Handoff;
use crate::balance::LiveConnection;
use crate::balance::WorkerBalance;
use crate::config::PerThreadConfig;
use crate::listener::bind_reuseport;
use crate::shutdown::PerThreadShutdown;

pub(crate) fn worker_main(
  worker_id: usize,
  addr: SocketAddr,
  router: Arc<Router>,
  cfg: PerThreadConfig,
  shutdown: PerThreadShutdown,
  balance: Option<WorkerBalance>,
) -> Result<(), PerThreadError> {
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

      return Err(PerThreadError::new(
        worker_id,
        PerThreadErrorKind::Startup,
        e,
      ));
    }
  };

  let supervisor = shutdown.clone();
  // The accept loop runs as its own task, so the `LocalSet` re-polls only its
  // join handle, not the whole accept `select!`, each time a connection wakes.
  let accept_loop = async move {
    #[cfg(feature = "plugins")]
    if let Err(error) = router.setup_plugins_once() {
      return Err(PerThreadError::new(
        worker_id,
        PerThreadErrorKind::Startup,
        io::Error::other(error.clone()),
      ));
    }
    let mut backoff = Duration::from_millis(5);
    let semaphore = cfg
      .max_connections
      .map(|n| Arc::new(tokio::sync::Semaphore::new(n)));
    let (balancer, mut inbox) = match balance {
      Some(balance) => (Some(balance.balancer), Some(balance.inbox)),
      None => (None, None),
    };
    let listener = match bind_reuseport(addr, cfg.backlog) {
      Ok(listener) => {
        shutdown.report_bind_success(worker_id);
        Some(listener)
      }
      Err(e) if balancer.is_some() => {
        tracing::warn!("worker {worker_id}: bind failed, serving handed-over connections: {e}");
        shutdown.report_handoff_only(worker_id, e);
        None
      }
      Err(e) => {
        tracing::error!("worker {worker_id}: bind failed: {e}");
        return Err(PerThreadError::new(worker_id, PerThreadErrorKind::Bind, e));
      }
    };
    tracing::debug!("tako-pt worker {worker_id} listening on {addr}");

    #[cfg(feature = "signals")]
    let local_addr = listener
      .as_ref()
      .and_then(|listener| listener.local_addr().ok())
      .unwrap_or(addr)
      .to_string();
    #[cfg(feature = "signals")]
    tako_rs_core::signals::transport::emit_server_started(&local_addr, "tcp", false).await;

    let shutdown_fut = shutdown.notified();
    tokio::pin!(shutdown_fut);

    let mut connection_handles = tokio::task::JoinSet::new();
    let stopping = Arc::new(ShutdownSignal::default());
    let upgrades = UpgradeSupervisor::default();

    loop {
      let (stream, peer, live) = tokio::select! {
        accept = accept(listener.as_ref()) => {
          let (stream, peer) = match accept {
            Ok(v) => { backoff = Duration::from_millis(5); v },
            Err(e) => {
              tracing::warn!("worker {worker_id}: accept failed: {e}");
              tokio::select! {
                () = shutdown.notified() => break,
                () = tokio::time::sleep(backoff) => {},
              }
              backoff = (backoff * 2).min(Duration::from_secs(1));
              continue;
            }
          };
          if let Err(e) = stream.set_nodelay(true) {
            tracing::debug!("worker {worker_id}: set_nodelay failed for {peer}: {e}");
          }
          match &balancer {
            Some(balancer) => match balancer.keep_or_hand_off(worker_id, stream, peer) {
              Some((stream, live)) => (stream, peer, Some(live)),
              None => continue,
            },
            None => (stream, peer, None),
          }
        }
        Some((stream, peer)) = next_handoff(inbox.as_mut()) => {
          let live = balancer
            .as_ref()
            .map(|balancer| LiveConnection::new(Arc::clone(balancer), worker_id));
          match TcpStream::from_std(stream) {
            Ok(stream) => (stream, peer, live),
            Err(e) => {
              tracing::debug!("worker {worker_id}: could not adopt {peer}: {e}");
              continue;
            }
          }
        }
        () = &mut shutdown_fut => {
          tracing::info!("worker {worker_id}: shutdown signalled, draining");
          break;
        }
      };

      let permit = if let Some(semaphore) = &semaphore {
        tokio::select! {
          () = shutdown.notified() => break,
          permit = semaphore.clone().acquire_owned() => permit.ok(),
        }
      } else {
        None
      };
      let ctx = Rc::new(ConnCtx::new(Arc::clone(&router), peer).with_upgrades(upgrades.registry()));
      let io = Http1Io::new(
        hyper_util::rt::TokioIo::new(Leased::new(stream, (live, permit))),
        Arc::clone(&ctx.activity),
      );
      let stopping = Arc::clone(&stopping);
      let header_read_timeout = cfg.header_read_timeout;

      connection_handles.spawn_local(async move {
        let mut http = http1::Builder::new();
        http.keep_alive(true);
        http.pipeline_flush(true);
        let conn = http
          .serve_connection(io, tcp_service(Rc::clone(&ctx)))
          .with_upgrades();
        let driven = ConnDriver::new(
          conn,
          http1::UpgradeableConnection::graceful_shutdown,
          stopping,
          ctx,
          header_read_timeout,
        );
        match driven.await {
          Some(Err(err)) if err.is_incomplete_message() => {
            tracing::debug!("worker {worker_id}: client disconnected mid-message: {err}");
          }
          Some(Err(err)) => tracing::error!("worker {worker_id}: connection error: {err}"),
          Some(Ok(())) => {}
          None => tracing::debug!("worker {worker_id}: closed {peer} at the header deadline"),
        }
      });

      while connection_handles.try_join_next().is_some() {}
    }

    stopping.trigger();
    upgrades.trigger();
    let drain = tokio::time::timeout(cfg.drain_timeout, async {
      while connection_handles.join_next().await.is_some() {}
      upgrades.join().await;
    });
    if drain.await.is_err() {
      upgrades.abort();
      connection_handles.shutdown().await;
      upgrades.join().await;
    }
    #[cfg(feature = "signals")]
    tako_rs_core::signals::transport::emit_server_stopped(&local_addr, "tcp", false).await;
    Ok(())
  };
  let local = LocalSet::new();
  supervisor.supervise_runtime(worker_id, || {
    local.block_on(&rt, async move {
      match tokio::task::spawn_local(accept_loop).await {
        Ok(result) => result,
        Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        Err(error) => Err(PerThreadError::new(
          worker_id,
          PerThreadErrorKind::Io,
          io::Error::other(error),
        )),
      }
    })
  })
}

async fn accept(listener: Option<&TcpListener>) -> io::Result<(TcpStream, SocketAddr)> {
  match listener {
    Some(listener) => listener.accept().await,
    None => std::future::pending().await,
  }
}

async fn next_handoff(inbox: Option<&mut mpsc::UnboundedReceiver<Handoff>>) -> Option<Handoff> {
  match inbox {
    Some(inbox) => inbox.recv().await,
    None => std::future::pending().await,
  }
}
