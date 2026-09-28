#![cfg_attr(docsrs, feature(doc_cfg))]

//! Thread-per-core HTTP server bootstrap for the Tako framework.
//!
//! Spawns OS threads with `SO_REUSEPORT` listeners. Each worker runs a Tokio
//! current-thread runtime by default, or Compio when `compio` is enabled.
//! The thread-safe router is shared; connections remain on their worker thread.
//! Tokio workers hand new connections to the least busy worker when
//! [`PerThreadConfig::balance_connections`] is set, which it is by default.
//!
//! [`serve_per_thread`] waits for process shutdown. [`spawn_per_thread`] returns
//! worker handles and a shutdown trigger for explicit control.
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

#[cfg(not(feature = "compio"))]
mod balance;
mod config;
mod listener;
mod shutdown;
#[cfg(not(feature = "compio"))]
mod worker;
#[cfg(feature = "compio")]
mod worker_compio;

use std::io;
use std::net::SocketAddr;
use std::str::FromStr;

use tako_rs_core::router::Router;

pub use crate::config::PerThreadConfig;
pub use crate::shutdown::PerThreadShutdown;
#[cfg(not(feature = "compio"))]
use crate::worker::worker_main;
#[cfg(feature = "compio")]
use crate::worker_compio::worker_main_compio;

/// Runs workers on the selected runtime until Ctrl+C or SIGTERM, then drains requests.
pub fn serve_per_thread(addr: &str, router: Router, cfg: PerThreadConfig) -> io::Result<()> {
  let workers = cfg.workers;
  let (handles, shutdown) = spawn_per_thread(addr, router, cfg)?;
  wait_for_shutdown(handles, shutdown, workers)
}

/// Starts workers on the selected runtime and returns handles for explicit shutdown.
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

/// Runs Compio workers until Ctrl+C or SIGTERM, then drains active requests.
#[cfg(feature = "compio")]
pub fn serve_per_thread_compio(addr: &str, router: Router, cfg: PerThreadConfig) -> io::Result<()> {
  let workers = cfg.workers;
  let (handles, shutdown) = spawn_per_thread_compio(addr, router, cfg)?;
  wait_for_shutdown(handles, shutdown, workers)
}

/// Starts Compio workers and returns handles for explicit shutdown control.
#[cfg(feature = "compio")]
pub fn spawn_per_thread_compio(
  addr: &str,
  router: Router,
  cfg: PerThreadConfig,
) -> io::Result<(Vec<std::thread::JoinHandle<()>>, PerThreadShutdown)> {
  spawn_workers(addr, router, cfg, worker_main_compio)
}

#[cfg(not(feature = "compio"))]
type WorkerFn = fn(
  usize,
  SocketAddr,
  std::sync::Arc<Router>,
  PerThreadConfig,
  PerThreadShutdown,
  Option<balance::WorkerBalance>,
);
#[cfg(feature = "compio")]
type WorkerFn = fn(usize, SocketAddr, std::sync::Arc<Router>, PerThreadConfig, PerThreadShutdown);

fn spawn_workers(
  addr: &str,
  router: Router,
  cfg: PerThreadConfig,
  worker: WorkerFn,
) -> io::Result<(Vec<std::thread::JoinHandle<()>>, PerThreadShutdown)> {
  if cfg.workers == 0 || cfg.max_connections == Some(0) {
    return Err(io::Error::new(
      io::ErrorKind::InvalidInput,
      "worker and connection counts must be positive",
    ));
  }
  let addr =
    SocketAddr::from_str(addr).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
  let router = std::sync::Arc::new(router);
  if !cfg.balance_connections || cfg!(feature = "compio") {
    listener::warn_reuseport_platform_once();
  }
  let shutdown = PerThreadShutdown::new();
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
      #[cfg(not(feature = "compio"))]
      worker(id, addr, router, cfg, worker_shutdown, balance);
      #[cfg(feature = "compio")]
      worker(id, addr, router, cfg, worker_shutdown);
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

fn wait_for_shutdown(
  handles: Vec<std::thread::JoinHandle<()>>,
  shutdown: PerThreadShutdown,
  workers: usize,
) -> io::Result<()> {
  let result = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .and_then(|rt| {
      rt.block_on(async {
        shutdown.wait_for_bind_outcome(workers).await?;
        tako_rs_core::server_support::shutdown_signal().await
      })
    });
  shutdown.trigger();
  for handle in handles {
    let _ = handle.join();
  }
  result
}
