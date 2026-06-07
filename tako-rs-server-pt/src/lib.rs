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

/// Starts a thread-per-core HTTP server with the given router.
///
/// Spawns `cfg.workers` OS threads. Each worker binds its own `SO_REUSEPORT`
/// socket on `addr`, builds a single-threaded tokio runtime, and serves
/// connections via [`tokio::task::spawn_local`].
///
/// This blocks the calling thread until all workers exit. To control shutdown
/// externally use [`spawn_per_thread`] which returns a [`PerThreadShutdown`]
/// handle.
pub fn serve_per_thread(addr: &str, router: Router, cfg: PerThreadConfig) -> io::Result<()> {
  let workers = cfg.workers;
  let (handle, shutdown) = spawn_per_thread(addr, router, cfg)?;
  // Wait for SIGINT (Ctrl+C) on a dedicated mini-runtime and then trigger
  // graceful shutdown. The earlier `drop(shutdown)` was a no-op — dropping
  // one clone of the `CancellationToken` does not cancel anything; only
  // `trigger()` does. Without this, the function would never return on a
  // healthy process.
  let rt = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .map_err(|e| io::Error::other(format!("ctrl-c runtime: {e}")))?;
  // Block on bind-outcome first: if every worker failed to bind
  // (SO_REUSEPORT unavailable, port already taken, …) we surface the first
  // recorded `io::Error` instead of pretending the server is up and waiting
  // forever on Ctrl+C. If at least one worker bound successfully, proceed
  // to the Ctrl+C wait as usual.
  let result: io::Result<()> = rt.block_on(async {
    shutdown.wait_for_bind_outcome(workers).await?;
    let _ = tokio::signal::ctrl_c().await;
    Ok(())
  });
  shutdown.trigger();
  for h in handle {
    let _ = h.join();
  }
  result
}

/// Spawn the worker threads and return both the join handles and a
/// [`PerThreadShutdown`] that the caller can use to signal a clean stop.
///
/// The returned thread handles are owned by the caller; dropping them does not
/// stop the server. Trigger the shutdown via [`PerThreadShutdown::trigger`],
/// then `join` each handle (or just drop them after the trigger if you're OK
/// with detached cleanup).
pub fn spawn_per_thread(
  addr: &str,
  router: Router,
  cfg: PerThreadConfig,
) -> io::Result<(Vec<std::thread::JoinHandle<()>>, PerThreadShutdown)> {
  let socket_addr =
    SocketAddr::from_str(addr).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

  // Leak the router so workers share a `&'static` reference — no Arc clones
  // on the per-connection or per-request hot path.
  let router: &'static Router = Box::leak(Box::new(router));

  let shutdown = PerThreadShutdown::new();
  let mut handles = Vec::with_capacity(cfg.workers);
  for worker_id in 0..cfg.workers {
    let cfg = cfg.clone();
    let shutdown = shutdown.clone();
    let h = std::thread::Builder::new()
      .name(format!("tako-pt-{worker_id}"))
      .spawn(move || worker_main(worker_id, socket_addr, router, cfg, shutdown))
      .expect("spawn tako-pt worker");
    handles.push(h);
  }
  Ok((handles, shutdown))
}

/// Starts a thread-per-core HTTP server with the compio runtime.
///
/// Same `SO_REUSEPORT` bootstrap as [`serve_per_thread`] but each worker runs a
/// single-threaded `compio` runtime — `io_uring` on Linux, IOCP on Windows,
/// kqueue on macOS. The router type stays the standard thread-safe
/// [`tako_rs_core::router::Router`].
#[cfg(feature = "compio")]
#[cfg_attr(docsrs, doc(cfg(feature = "compio")))]
pub fn serve_per_thread_compio(addr: &str, router: Router, cfg: PerThreadConfig) -> io::Result<()> {
  let socket_addr =
    SocketAddr::from_str(addr).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

  let router: &'static Router = Box::leak(Box::new(router));

  let workers = cfg.workers;
  let shutdown = PerThreadShutdown::new();
  let mut handles = Vec::with_capacity(cfg.workers);
  for worker_id in 0..cfg.workers {
    let cfg = cfg.clone();
    let shutdown = shutdown.clone();
    let h = std::thread::Builder::new()
      .name(format!("tako-pt-compio-{worker_id}"))
      .spawn(move || worker_main_compio(worker_id, socket_addr, router, cfg, shutdown))
      .expect("spawn tako-pt-compio worker");
    handles.push(h);
  }

  // Same Ctrl+C / shutdown discipline as `serve_per_thread`, plus the same
  // bind-outcome wait so an all-bind-fail does not silently look healthy.
  let rt = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .map_err(|e| io::Error::other(format!("ctrl-c runtime: {e}")))?;
  let result: io::Result<()> = rt.block_on(async {
    shutdown.wait_for_bind_outcome(workers).await?;
    let _ = tokio::signal::ctrl_c().await;
    Ok(())
  });
  shutdown.trigger();
  for h in handles {
    let _ = h.join();
  }
  result
}
