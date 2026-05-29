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

use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use socket2::Domain;
use socket2::Protocol;
use socket2::Socket;
use socket2::Type;
use tako_rs_core::body::TakoBody;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::router::Router;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::task::LocalSet;

/// Configuration for [`serve_per_thread`] (and the `compio` variant when enabled).
#[derive(Debug, Clone)]
pub struct PerThreadConfig {
  /// Number of worker threads. Defaults to the number of logical CPUs.
  pub workers: usize,
  /// Pin each worker to a CPU core (requires the `affinity` feature).
  pub pin_to_core: bool,
  /// `SO_REUSEPORT` listen backlog.
  pub backlog: i32,
  /// Maximum time the coordinator waits for in-flight requests after shutdown.
  /// Workers are dropped after this elapses.
  pub drain_timeout: Duration,
}

impl Default for PerThreadConfig {
  fn default() -> Self {
    Self {
      workers: num_cpus(),
      pin_to_core: cfg!(feature = "affinity"),
      backlog: 1024,
      drain_timeout: Duration::from_secs(30),
    }
  }
}

/// Shared bind-result tracker used by the [`PerThreadShutdown`] coordinator
/// so the parent process can detect "all worker threads failed to bind"
/// (e.g. `SO_REUSEPORT` unavailable on Windows / non-Linux Unix, port already
/// taken) and surface a real error from [`serve_per_thread`] instead of
/// silently waiting on Ctrl+C forever and then returning `Ok(())`.
#[derive(Default)]
struct BindStatus {
  /// Number of workers that completed their bind step successfully.
  succeeded: std::sync::atomic::AtomicUsize,
  /// Number of workers that failed their bind step.
  failed: std::sync::atomic::AtomicUsize,
  /// First recorded bind error so the parent can return something
  /// actionable to its caller / supervisor. Plain `std::sync::Mutex` is
  /// fine here — this is a cold path (one write per worker at startup,
  /// one read on shutdown).
  first_err: std::sync::Mutex<Option<io::Error>>,
  /// Wake-up notify so the parent does not have to poll.
  notify: tokio::sync::Notify,
}

/// Shutdown coordinator shared by every worker spawned via [`spawn_per_thread`]
/// (and friends). Workers `select!` against [`Self::notified`] in their accept
/// loop, so triggering [`PerThreadShutdown::trigger`] cleanly exits each
/// worker's `loop { accept }` instead of leaking the OS thread on shutdown.
///
/// Backed by a [`tokio_util::sync::CancellationToken`] so the signal is
/// sticky: workers that register `notified()` after `trigger()` was called
/// still observe the request immediately, fixing the `Notify::notify_waiters`
/// race where late subscribers would miss the shutdown.
///
/// Also carries a private [`BindStatus`] that workers update with the result
/// of their `SO_REUSEPORT` bind so the parent (e.g. [`serve_per_thread`]) can
/// fail loudly on "every worker failed to bind" instead of returning Ok(()) —
/// previously the function would await Ctrl+C indefinitely and then claim
/// success even when no listener was up, a false health signal to supervisors.
#[derive(Clone, Default)]
pub struct PerThreadShutdown {
  inner: tokio_util::sync::CancellationToken,
  bind_status: std::sync::Arc<BindStatus>,
}

impl PerThreadShutdown {
  /// Construct an unsignalled shutdown coordinator.
  #[must_use]
  pub fn new() -> Self {
    Self::default()
  }

  /// Notify every worker waiter that it should exit its accept loop.
  /// Idempotent — calling it more than once is a no-op.
  pub fn trigger(&self) {
    self.inner.cancel();
  }

  /// Future a worker awaits to learn that shutdown has been requested.
  pub async fn notified(&self) {
    self.inner.cancelled().await;
  }

  /// Worker hook: report a successful `SO_REUSEPORT` bind.
  pub(crate) fn report_bind_success(&self) {
    self
      .bind_status
      .succeeded
      .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    self.bind_status.notify.notify_waiters();
  }

  /// Worker hook: report a bind failure (with the underlying `io::Error`).
  /// The first error wins for reporting; later errors are dropped after their
  /// `tracing::error!` log.
  pub(crate) fn report_bind_failure(&self, err: io::Error) {
    {
      let mut guard = self.bind_status.first_err.lock().unwrap();
      if guard.is_none() {
        *guard = Some(err);
      }
    }
    self
      .bind_status
      .failed
      .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    self.bind_status.notify.notify_waiters();
  }

  /// Wait until either at least one worker bound successfully, or all
  /// `total` workers reported a bind failure. Returns the first recorded
  /// `io::Error` in the all-failure case so the caller can propagate a real
  /// error instead of pretending the server started.
  pub async fn wait_for_bind_outcome(&self, total: usize) -> io::Result<()> {
    use std::sync::atomic::Ordering;

    loop {
      // Arm the notified future BEFORE reading state so a wake fired
      // between the load and the await is not lost.
      let notified = self.bind_status.notify.notified();
      tokio::pin!(notified);
      notified.as_mut().enable();

      let succ = self.bind_status.succeeded.load(Ordering::SeqCst);
      let fail = self.bind_status.failed.load(Ordering::SeqCst);

      if succ > 0 {
        return Ok(());
      }
      if succ + fail >= total {
        let err = self
          .bind_status
          .first_err
          .lock()
          .unwrap()
          .take()
          .unwrap_or_else(|| {
            io::Error::other(format!("all {total} per-thread workers failed to bind"))
          });
        return Err(err);
      }

      notified.await;
    }
  }
}

fn num_cpus() -> usize {
  std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
}

#[cfg(feature = "compio")]
fn compio_accept_backoff() -> Duration {
  Duration::from_millis(5)
}

/// One-shot platform-capability warning. `SO_REUSEPORT` behaves like
/// kernel-level load balancing only on Linux; macOS / *BSD ignore the load-
/// balance semantic (last-binder-wins), and Windows lacks the option entirely.
fn warn_reuseport_platform_once() {
  static WARNED: std::sync::Once = std::sync::Once::new();
  WARNED.call_once(|| {
    #[cfg(target_os = "linux")]
    {
      // No-op: SO_REUSEPORT is the supported configuration.
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
      tracing::warn!(
        "tako-server-pt: SO_REUSEPORT is being used on a non-Linux Unix \
         platform. The kernel typically sends incoming connections only to \
         the most recent binder, so multi-worker thread-per-core mode will \
         not load-balance correctly. Use a single worker or run on Linux."
      );
    }
    #[cfg(windows)]
    {
      tracing::warn!(
        "tako-server-pt: SO_REUSEPORT does not exist on Windows. Only the \
         first worker will accept connections; subsequent worker binds will \
         fail with EADDRINUSE. Use a single worker on Windows."
      );
    }
  });
}

fn bind_reuseport_std(addr: SocketAddr, backlog: i32) -> io::Result<std::net::TcpListener> {
  warn_reuseport_platform_once();
  let domain = if addr.is_ipv4() {
    Domain::IPV4
  } else {
    Domain::IPV6
  };
  let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
  socket.set_reuse_address(true)?;
  // `socket2::set_reuse_port` is gated to Unix targets only; on Linux it's a
  // genuine kernel load-balancer, on macOS / BSD it's a no-op-equivalent
  // (last-binder-wins), on Windows the underlying SO_REUSEPORT does not
  // exist so the call is omitted entirely.
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
fn bind_reuseport_compio(addr: SocketAddr, backlog: i32) -> io::Result<compio::net::TcpListener> {
  compio::net::TcpListener::from_std(bind_reuseport_std(addr, backlog)?)
}

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

// Without the `affinity` feature, `worker_id` and `cfg.pin_to_core` aren't
// read past this point; mark the function tolerant of those unused names so
// we don't need the awkward `let _ = (worker_id, &cfg.pin_to_core);` trick
// that previously sat inside the function body for the sole purpose of
// silencing the warning.
#[cfg_attr(not(feature = "affinity"), allow(unused_variables))]
fn worker_main(
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
fn worker_main_compio(
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
