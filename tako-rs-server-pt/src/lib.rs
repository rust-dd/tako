#![cfg_attr(docsrs, feature(doc_cfg))]

//! Thread-per-core HTTP/1 server: every worker thread runs its own `SO_REUSEPORT` listener on a
//! Tokio current-thread runtime, or compio with `compio`, and all workers share one router.
//!
//! ```no_run
//! use tako_rs_server_pt::{PerThreadConfig, serve_per_thread};
//! use tako_rs_core::router::Router;
//!
//! fn main() -> std::io::Result<()> {
//!     let mut router = Router::new();
//!     router.get("/", || async { "hello" });
//!     let config = PerThreadConfig {
//!         workers: 4,
//!         pin_to_core: false,
//!         ..Default::default()
//!     };
//!     serve_per_thread("0.0.0.0:8080", router, config)
//! }
//! ```
//!
//! [`spawn_per_thread_supervised`] returns a [`PerThreadHandle`] for readiness, shutdown,
//! and worker failure reporting.

#[cfg(not(feature = "compio"))]
mod balance;
mod config;
mod error;
mod handle;
mod listener;
mod readiness;
mod shutdown;
#[cfg(not(feature = "compio"))]
mod worker;
#[cfg(feature = "compio")]
mod worker_compio;

#[cfg(test)]
mod supervision_tests;

use std::io;
use std::net::SocketAddr;
use std::str::FromStr;

use tako_rs_core::router::Router;

pub use crate::config::PerThreadConfig;
pub use crate::error::PerThreadError;
pub use crate::error::PerThreadErrorKind;
pub use crate::handle::PerThreadHandle;
pub use crate::readiness::PerThreadReadiness;
pub use crate::readiness::PerThreadWorkerState;
pub use crate::shutdown::PerThreadShutdown;
#[cfg(not(feature = "compio"))]
use crate::worker::worker_main;
#[cfg(feature = "compio")]
use crate::worker_compio::worker_main_compio;

/// Runs until Ctrl+C, SIGTERM or worker failure, then drains and reports the terminal result.
pub fn serve_per_thread(addr: &str, router: Router, cfg: PerThreadConfig) -> io::Result<()> {
  wait_for_shutdown(spawn_per_thread_supervised(addr, router, cfg)?)
}

/// Starts workers and returns their thread handles and the trigger that stops them.
/// Worker failures cancel peers; use [`spawn_per_thread_supervised`] to observe the failure.
pub fn spawn_per_thread(
  addr: &str,
  router: Router,
  cfg: PerThreadConfig,
) -> io::Result<(Vec<std::thread::JoinHandle<()>>, PerThreadShutdown)> {
  #[cfg(not(feature = "compio"))]
  {
    spawn_workers(addr, router, cfg, worker_main)
  }
  #[cfg(feature = "compio")]
  {
    spawn_workers(addr, router, cfg, worker_main_compio)
  }
}

/// Starts supervised workers. Validation and thread creation errors return directly;
/// worker failures reach the handle and stop peers.
pub fn spawn_per_thread_supervised(
  addr: &str,
  router: Router,
  cfg: PerThreadConfig,
) -> io::Result<PerThreadHandle> {
  let (workers, shutdown) = spawn_per_thread(addr, router, cfg)?;
  Ok(PerThreadHandle::new(workers, shutdown))
}

#[cfg(not(feature = "compio"))]
type WorkerFn = fn(
  usize,
  SocketAddr,
  std::sync::Arc<Router>,
  PerThreadConfig,
  PerThreadShutdown,
  Option<balance::WorkerBalance>,
) -> Result<(), PerThreadError>;
#[cfg(feature = "compio")]
type WorkerFn = fn(
  usize,
  SocketAddr,
  std::sync::Arc<Router>,
  PerThreadConfig,
  PerThreadShutdown,
) -> Result<(), PerThreadError>;

fn spawn_workers(
  addr: &str,
  router: Router,
  cfg: PerThreadConfig,
  worker: WorkerFn,
) -> io::Result<(Vec<std::thread::JoinHandle<()>>, PerThreadShutdown)> {
  cfg.validate()?;

  let addr =
    SocketAddr::from_str(addr).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
  let router = std::sync::Arc::new(router);
  if !cfg.balance_connections || cfg!(feature = "compio") {
    listener::warn_reuseport_platform_once();
  }

  let shutdown = PerThreadShutdown::new(cfg.workers);
  #[cfg(not(feature = "compio"))]
  let mut balances = cfg
    .balance_connections
    .then(|| balance::Balancer::for_workers(cfg.workers).into_iter());
  let mut handles = Vec::with_capacity(cfg.workers);

  for id in 0..cfg.workers {
    let cfg = cfg.clone();
    let router = router.clone();
    let worker_shutdown = shutdown.clone();
    #[cfg(not(feature = "compio"))]
    let balance = balances.as_mut().and_then(Iterator::next);
    let run = move || {
      let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        #[cfg(not(feature = "compio"))]
        {
          worker(id, addr, router, cfg, worker_shutdown.clone(), balance)
        }
        #[cfg(feature = "compio")]
        {
          worker(id, addr, router, cfg, worker_shutdown.clone())
        }
      }));

      match outcome {
        Ok(result) => worker_shutdown.report_exit(id, result),
        Err(payload) => {
          worker_shutdown.report_exit(id, Err(PerThreadError::panic(id, payload.as_ref())));
          std::panic::resume_unwind(payload);
        }
      }
    };

    match std::thread::Builder::new()
      .name(format!("tako-pt-{id}"))
      .spawn(run)
    {
      Ok(handle) => handles.push(handle),
      Err(error) => {
        shutdown.trigger();
        for handle in handles {
          let _ = handle.join();
        }

        return Err(error);
      }
    }
  }

  Ok((handles, shutdown))
}

fn wait_for_shutdown(handle: PerThreadHandle) -> io::Result<()> {
  let result = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .and_then(|rt| {
      rt.block_on(async {
        tokio::select! {
          result = handle.result() => result.map_err(io::Error::from),
          signal = tako_rs_core::server_support::shutdown_signal() => signal,
        }
      })
    });
  handle.shutdown();
  let joined = handle.join().map_err(io::Error::from);
  result.and(joined)
}
