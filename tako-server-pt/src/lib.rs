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
//! - [`serve_per_thread`] — uses the existing thread-safe [`tako::router::Router`]
//!   from `tako-core`. Drop-in alternative to [`tako::serve`]; no API changes.
//! - [`serve_per_thread_local`] (under the `local` feature) — uses
//!   [`tako_core_local::router::LocalRouter`]. Allows `!Send` handler state
//!   such as `Rc<RefCell<…>>` per worker.
//! - [`serve_per_thread_compio`] (under the `compio` feature) — same SO_REUSEPORT
//!   bootstrap but each worker runs a `compio` runtime (io_uring on Linux,
//!   IOCP on Windows, kqueue on macOS).
//! - [`serve_per_thread_compio_local`] (under the `compio-local` feature) —
//!   compio runtime + `LocalRouter` per worker.

use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::str::FromStr;
#[cfg(any(feature = "local", feature = "compio-local"))]
use std::sync::Arc;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use socket2::Domain;
use socket2::Protocol;
use socket2::Socket;
use socket2::Type;
use tako_core::body::TakoBody;
use tako_core::router::Router;
#[cfg(feature = "local")]
use tako_core_local::router::LocalRouter;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::task::LocalSet;

/// Configuration for [`serve_per_thread`] and [`serve_per_thread_local`].
#[derive(Debug, Clone)]
pub struct PerThreadConfig {
  /// Number of worker threads. Defaults to the number of logical CPUs.
  pub workers: usize,
  /// Pin each worker to a CPU core (requires the `affinity` feature).
  pub pin_to_core: bool,
  /// `SO_REUSEPORT` listen backlog.
  pub backlog: i32,
}

impl Default for PerThreadConfig {
  fn default() -> Self {
    Self {
      workers: num_cpus(),
      pin_to_core: cfg!(feature = "affinity"),
      backlog: 1024,
    }
  }
}

fn num_cpus() -> usize {
  std::thread::available_parallelism()
    .map(|n| n.get())
    .unwrap_or(1)
}

fn bind_reuseport_std(addr: SocketAddr, backlog: i32) -> io::Result<std::net::TcpListener> {
  let domain = if addr.is_ipv4() {
    Domain::IPV4
  } else {
    Domain::IPV6
  };
  let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
  socket.set_reuse_address(true)?;
  #[cfg(unix)]
  socket.set_reuse_port(true)?;
  socket.set_nonblocking(true)?;
  socket.bind(&addr.into())?;
  socket.listen(backlog)?;
  Ok(socket.into())
}

fn bind_reuseport(addr: SocketAddr, backlog: i32) -> io::Result<TcpListener> {
  TcpListener::from_std(bind_reuseport_std(addr, backlog)?)
}

#[cfg(feature = "compio")]
fn bind_reuseport_compio(
  addr: SocketAddr,
  backlog: i32,
) -> io::Result<compio::net::TcpListener> {
  compio::net::TcpListener::from_std(bind_reuseport_std(addr, backlog)?)
}

/// Starts a thread-per-core HTTP server with the given router.
///
/// Spawns `cfg.workers` OS threads. Each worker binds its own `SO_REUSEPORT`
/// socket on `addr`, builds a single-threaded tokio runtime, and serves
/// connections via [`tokio::task::spawn_local`].
///
/// This blocks the calling thread until all workers exit.
pub fn serve_per_thread(addr: &str, router: Router, cfg: PerThreadConfig) -> io::Result<()> {
  let socket_addr =
    SocketAddr::from_str(addr).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

  // Leak the router so workers share a `&'static` reference — no Arc clones
  // on the per-connection or per-request hot path.
  let router: &'static Router = Box::leak(Box::new(router));

  let mut handles = Vec::with_capacity(cfg.workers);
  for worker_id in 0..cfg.workers {
    let cfg = cfg.clone();
    let h = std::thread::Builder::new()
      .name(format!("tako-pt-{worker_id}"))
      .spawn(move || worker_main(worker_id, socket_addr, router, cfg))
      .expect("spawn tako-pt worker");
    handles.push(h);
  }

  for h in handles {
    let _ = h.join();
  }
  Ok(())
}

fn worker_main(worker_id: usize, addr: SocketAddr, router: &'static Router, cfg: PerThreadConfig) {
  #[cfg(feature = "affinity")]
  if cfg.pin_to_core {
    if let Some(ids) = core_affinity::get_core_ids() {
      if let Some(id) = ids.get(worker_id) {
        let _ = core_affinity::set_for_current(*id);
      }
    }
  }
  let _ = (worker_id, &cfg.pin_to_core);

  let rt = match Builder::new_current_thread().enable_all().build() {
    Ok(rt) => rt,
    Err(e) => {
      tracing::error!("worker {worker_id}: failed to build runtime: {e}");
      return;
    }
  };

  let local = LocalSet::new();
  local.block_on(&rt, async move {
    let listener = match bind_reuseport(addr, cfg.backlog) {
      Ok(l) => l,
      Err(e) => {
        tracing::error!("worker {worker_id}: bind failed: {e}");
        return;
      }
    };
    tracing::debug!("tako-pt worker {worker_id} listening on {addr}");

    loop {
      let accept = match listener.accept().await {
        Ok(v) => v,
        Err(e) => {
          tracing::error!("worker {worker_id}: accept failed: {e}");
          continue;
        }
      };
      let (stream, peer) = accept;
      let _ = stream.set_nodelay(true);
      let io = hyper_util::rt::TokioIo::new(stream);

      tokio::task::spawn_local(async move {
        let svc = service_fn(move |mut req| async move {
          req.extensions_mut().insert(peer);
          let resp = router.dispatch(req.map(TakoBody::incoming)).await;
          Ok::<_, Infallible>(resp)
        });

        let mut http = http1::Builder::new();
        http.keep_alive(true);
        http.pipeline_flush(true);
        if let Err(err) = http.serve_connection(io, svc).with_upgrades().await {
          tracing::error!("worker {worker_id}: connection error: {err}");
        }
      });
    }
  });
}

/// Starts a thread-per-core HTTP server using the `!Send` [`LocalRouter`].
///
/// Same bootstrap as [`serve_per_thread`] but uses the local router from
/// `tako-core-local`, allowing handlers to capture `!Send` state such as
/// `Rc<RefCell<…>>` that lives entirely on a single worker thread.
///
/// `LocalRouter` cannot cross thread boundaries; the workers each construct
/// their own router via the `init` callback.
#[cfg(feature = "local")]
#[cfg_attr(docsrs, doc(cfg(feature = "local")))]
pub fn serve_per_thread_local<F>(addr: &str, init: F, cfg: PerThreadConfig) -> io::Result<()>
where
  F: Fn() -> LocalRouter + Send + Sync + 'static,
{
  use std::convert::Infallible;

  let socket_addr =
    SocketAddr::from_str(addr).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

  let init: Arc<dyn Fn() -> LocalRouter + Send + Sync + 'static> = Arc::new(init);

  let mut handles = Vec::with_capacity(cfg.workers);
  for worker_id in 0..cfg.workers {
    let cfg = cfg.clone();
    let init = Arc::clone(&init);
    let h = std::thread::Builder::new()
      .name(format!("tako-pt-local-{worker_id}"))
      .spawn(move || {
        #[cfg(feature = "affinity")]
        if cfg.pin_to_core {
          if let Some(ids) = core_affinity::get_core_ids() {
            if let Some(id) = ids.get(worker_id) {
              let _ = core_affinity::set_for_current(*id);
            }
          }
        }

        let rt = match Builder::new_current_thread().enable_all().build() {
          Ok(rt) => rt,
          Err(e) => {
            tracing::error!("worker {worker_id}: failed to build runtime: {e}");
            return;
          }
        };

        let local_set = LocalSet::new();
        local_set.block_on(&rt, async move {
          let router = init();
          let router = std::rc::Rc::new(router);

          let listener = match bind_reuseport(socket_addr, cfg.backlog) {
            Ok(l) => l,
            Err(e) => {
              tracing::error!("worker {worker_id}: bind failed: {e}");
              return;
            }
          };
          tracing::debug!("tako-pt-local worker {worker_id} listening on {socket_addr}");

          loop {
            let accept = match listener.accept().await {
              Ok(v) => v,
              Err(e) => {
                tracing::error!("worker {worker_id}: accept failed: {e}");
                continue;
              }
            };
            let (stream, peer) = accept;
            let _ = stream.set_nodelay(true);
            let io = hyper_util::rt::TokioIo::new(stream);
            let router = std::rc::Rc::clone(&router);

            tokio::task::spawn_local(async move {
              let svc = service_fn(move |mut req| {
                let router = std::rc::Rc::clone(&router);
                async move {
                  req.extensions_mut().insert(peer);
                  let resp = router.dispatch(req.map(TakoBody::incoming)).await;
                  Ok::<_, Infallible>(resp)
                }
              });

              let mut http = http1::Builder::new();
              http.keep_alive(true);
              http.pipeline_flush(true);
              if let Err(err) = http.serve_connection(io, svc).with_upgrades().await {
                tracing::error!("worker {worker_id}: connection error: {err}");
              }
            });
          }
        });
      })
      .expect("spawn tako-pt-local worker");
    handles.push(h);
  }

  for h in handles {
    let _ = h.join();
  }
  Ok(())
}

/// Starts a thread-per-core HTTP server with the compio runtime.
///
/// Same SO_REUSEPORT bootstrap as [`serve_per_thread`] but each worker runs a
/// single-threaded `compio` runtime — io_uring on Linux, IOCP on Windows,
/// kqueue on macOS. The router type stays the standard thread-safe
/// [`tako::router::Router`].
#[cfg(feature = "compio")]
#[cfg_attr(docsrs, doc(cfg(feature = "compio")))]
pub fn serve_per_thread_compio(
  addr: &str,
  router: Router,
  cfg: PerThreadConfig,
) -> io::Result<()> {
  let socket_addr =
    SocketAddr::from_str(addr).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

  let router: &'static Router = Box::leak(Box::new(router));

  let mut handles = Vec::with_capacity(cfg.workers);
  for worker_id in 0..cfg.workers {
    let cfg = cfg.clone();
    let h = std::thread::Builder::new()
      .name(format!("tako-pt-compio-{worker_id}"))
      .spawn(move || worker_main_compio(worker_id, socket_addr, router, cfg))
      .expect("spawn tako-pt-compio worker");
    handles.push(h);
  }

  for h in handles {
    let _ = h.join();
  }
  Ok(())
}

#[cfg(feature = "compio")]
fn worker_main_compio(
  worker_id: usize,
  addr: SocketAddr,
  router: &'static Router,
  cfg: PerThreadConfig,
) {
  use cyper_core::HyperStream;

  #[cfg(feature = "affinity")]
  if cfg.pin_to_core {
    if let Some(ids) = core_affinity::get_core_ids() {
      if let Some(id) = ids.get(worker_id) {
        let _ = core_affinity::set_for_current(*id);
      }
    }
  }
  let _ = (worker_id, &cfg.pin_to_core);

  let rt = match compio::runtime::RuntimeBuilder::new().build() {
    Ok(rt) => rt,
    Err(e) => {
      tracing::error!("worker {worker_id}: failed to build compio runtime: {e}");
      return;
    }
  };

  rt.block_on(async move {
    let listener = match bind_reuseport_compio(addr, cfg.backlog) {
      Ok(l) => l,
      Err(e) => {
        tracing::error!("worker {worker_id}: bind failed: {e}");
        return;
      }
    };
    tracing::debug!("tako-pt-compio worker {worker_id} listening on {addr}");

    loop {
      let accept = match listener.accept().await {
        Ok(v) => v,
        Err(e) => {
          tracing::error!("worker {worker_id}: accept failed: {e}");
          continue;
        }
      };
      let (stream, peer) = accept;
      let io = HyperStream::new(stream);

      compio::runtime::spawn(async move {
        let svc = service_fn(move |mut req| async move {
          req.extensions_mut().insert(peer);
          let resp = router
            .dispatch(req.map(tako_core::body::TakoBody::new))
            .await;
          Ok::<_, Infallible>(resp)
        });

        let mut http = http1::Builder::new();
        http.keep_alive(true);
        if let Err(err) = http.serve_connection(io, svc).with_upgrades().await {
          tracing::error!("worker {worker_id}: connection error: {err}");
        }
      })
      .detach();
    }
  });
}

/// Starts a thread-per-core HTTP server using compio + the `!Send`
/// [`LocalRouter`].
///
/// Each worker builds its own [`compio::runtime::Runtime`] and constructs a
/// `LocalRouter` via the `init` callback. compio's runtime is single-threaded
/// by design, so `!Send` futures and `Rc<RefCell<…>>` state work natively
/// without `LocalSet`.
#[cfg(feature = "compio-local")]
#[cfg_attr(docsrs, doc(cfg(feature = "compio-local")))]
pub fn serve_per_thread_compio_local<F>(
  addr: &str,
  init: F,
  cfg: PerThreadConfig,
) -> io::Result<()>
where
  F: Fn() -> LocalRouter + Send + Sync + 'static,
{
  use cyper_core::HyperStream;

  let socket_addr =
    SocketAddr::from_str(addr).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

  let init: Arc<dyn Fn() -> LocalRouter + Send + Sync + 'static> = Arc::new(init);

  let mut handles = Vec::with_capacity(cfg.workers);
  for worker_id in 0..cfg.workers {
    let cfg = cfg.clone();
    let init = Arc::clone(&init);
    let h = std::thread::Builder::new()
      .name(format!("tako-pt-compio-local-{worker_id}"))
      .spawn(move || {
        #[cfg(feature = "affinity")]
        if cfg.pin_to_core {
          if let Some(ids) = core_affinity::get_core_ids() {
            if let Some(id) = ids.get(worker_id) {
              let _ = core_affinity::set_for_current(*id);
            }
          }
        }

        let rt = match compio::runtime::RuntimeBuilder::new().build() {
          Ok(rt) => rt,
          Err(e) => {
            tracing::error!("worker {worker_id}: failed to build compio runtime: {e}");
            return;
          }
        };

        rt.block_on(async move {
          let router = std::rc::Rc::new(init());

          let listener = match bind_reuseport_compio(socket_addr, cfg.backlog) {
            Ok(l) => l,
            Err(e) => {
              tracing::error!("worker {worker_id}: bind failed: {e}");
              return;
            }
          };
          tracing::debug!(
            "tako-pt-compio-local worker {worker_id} listening on {socket_addr}"
          );

          loop {
            let accept = match listener.accept().await {
              Ok(v) => v,
              Err(e) => {
                tracing::error!("worker {worker_id}: accept failed: {e}");
                continue;
              }
            };
            let (stream, peer) = accept;
            let io = HyperStream::new(stream);
            let router = std::rc::Rc::clone(&router);

            compio::runtime::spawn(async move {
              let svc = service_fn(move |mut req| {
                let router = std::rc::Rc::clone(&router);
                async move {
                  req.extensions_mut().insert(peer);
                  let resp = router
                    .dispatch(req.map(tako_core::body::TakoBody::new))
                    .await;
                  Ok::<_, Infallible>(resp)
                }
              });

              let mut http = http1::Builder::new();
              http.keep_alive(true);
              if let Err(err) = http.serve_connection(io, svc).with_upgrades().await {
                tracing::error!("worker {worker_id}: connection error: {err}");
              }
            })
            .detach();
          }
        });
      })
      .expect("spawn tako-pt-compio-local worker");
    handles.push(h);
  }

  for h in handles {
    let _ = h.join();
  }
  Ok(())
}
