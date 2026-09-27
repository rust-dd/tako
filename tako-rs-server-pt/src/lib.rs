#![cfg_attr(docsrs, feature(doc_cfg))]

//! Thread-per-core HTTP server bootstrap for the Tako framework.
//!
//! Spawns N OS threads (one per CPU by default), each running its own
//! `tokio` `current_thread` runtime + [`tokio::task::LocalSet`]. Connections
//! are distributed across workers at the kernel level via `SO_REUSEPORT`.
//! Tasks never migrate between threads, eliminating tokio's work-stealing
//! coordination on the hot path and improving cache locality (especially with
//! the `affinity` feature which pins each worker to a specific core).
//!
//! Two entry points:
//!
//! - [`serve_per_thread`] — uses the existing thread-safe [`tako_rs_core::router::Router`]
//!   from `tako-core`. Drop-in alternative to `tako::serve`; no API changes.
//! - `serve_per_thread_compio` (under the `compio` feature) — same `SO_REUSEPORT`
//!   bootstrap but each worker runs a `compio` runtime (`io_uring` on Linux,
//!   IOCP on Windows, kqueue on macOS).

mod config;
mod listener;
mod shutdown;
mod worker;
#[cfg(feature = "compio")]
mod worker_compio;

use std::io;
use std::net::SocketAddr;
use std::str::FromStr;

use tako_rs_core::router::Router;

pub use crate::config::PerThreadConfig;
pub use crate::shutdown::PerThreadShutdown;
use crate::worker::worker_main;
#[cfg(feature = "compio")]
use crate::worker_compio::worker_main_compio;

/// Runs Tokio workers until Ctrl+C or SIGTERM, then drains active requests.
pub fn serve_per_thread(addr: &str, router: Router, cfg: PerThreadConfig) -> io::Result<()> {
  let workers = cfg.workers;
  let (handles, shutdown) = spawn_per_thread(addr, router, cfg)?;
  wait_for_shutdown(handles, shutdown, workers)
}

/// Starts Tokio workers and returns handles for explicit shutdown control.
pub fn spawn_per_thread(
  addr: &str,
  router: Router,
  cfg: PerThreadConfig,
) -> io::Result<(Vec<std::thread::JoinHandle<()>>, PerThreadShutdown)> {
  spawn_workers(addr, router, cfg, worker_main)
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

fn spawn_workers(
  addr: &str,
  router: Router,
  cfg: PerThreadConfig,
  worker: fn(usize, SocketAddr, std::sync::Arc<Router>, PerThreadConfig, PerThreadShutdown),
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
  let shutdown = PerThreadShutdown::new();
  let mut handles = Vec::with_capacity(cfg.workers);
  for id in 0..cfg.workers {
    let cfg = cfg.clone();
    let router = router.clone();
    let worker_shutdown = shutdown.clone();
    match std::thread::Builder::new()
      .name(format!("tako-pt-{id}"))
      .spawn(move || worker(id, addr, router, cfg, worker_shutdown))
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
