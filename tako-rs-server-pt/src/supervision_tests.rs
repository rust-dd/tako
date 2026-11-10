use std::time::Duration;

use tako_rs_core::router::Router;

use crate::PerThreadConfig;
use crate::PerThreadError;
use crate::PerThreadErrorKind;
use crate::PerThreadHandle;
use crate::PerThreadShutdown;
use crate::PerThreadWorkerState;
use crate::spawn_workers;

const WORKER_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Default)]
struct FailureControls {
  fail: tokio_util::sync::CancellationToken,
  peer_ready: tokio_util::sync::CancellationToken,
  release_peer: tokio_util::sync::CancellationToken,
}

fn post_startup_panic(id: usize, router: std::sync::Arc<Router>, shutdown: PerThreadShutdown) {
  let controls = router.router_state().get::<FailureControls>().unwrap();
  shutdown.report_bind_success(id);
  tokio::runtime::Builder::new_current_thread()
    .build()
    .unwrap()
    .block_on(async {
      if id == 0 {
        controls.fail.cancelled().await;
        panic!("failure after readiness");
      }

      controls.peer_ready.cancel();
      shutdown.notified().await;
      controls.release_peer.cancelled().await;
    });
}

#[tokio::test]
async fn post_startup_panic_clears_readiness_and_reports_before_peers_finish() {
  let controls = FailureControls::default();
  let mut router = Router::new();
  router.with_state(controls.clone());
  let (workers, shutdown) = spawn_workers(
    "127.0.0.1:0",
    router,
    PerThreadConfig {
      workers: 2,
      ..PerThreadConfig::default()
    },
    #[cfg(not(feature = "compio"))]
    |id, _, router, _, shutdown, _| {
      post_startup_panic(id, router, shutdown);
      Ok(())
    },
    #[cfg(feature = "compio")]
    |id, _, router, _, shutdown| {
      post_startup_panic(id, router, shutdown);
      Ok(())
    },
  )
  .unwrap();
  let handle = PerThreadHandle::new(workers, shutdown);
  let readiness = handle.readiness();
  handle.wait_ready().await.unwrap();
  controls.peer_ready.cancelled().await;
  assert!(readiness.is_ready());

  controls.fail.cancel();
  let result = tokio::time::timeout(WORKER_TIMEOUT, handle.result()).await;
  controls.release_peer.cancel();
  let error = result
    .expect("result must not wait for a draining peer")
    .unwrap_err();

  assert_eq!(error.kind(), PerThreadErrorKind::Panic);
  assert_eq!(error.worker_id(), 0);
  assert!(!readiness.is_ready());
  assert!(handle.wait_ready().await.is_err());
  assert_eq!(
    handle.result().await.unwrap_err().kind(),
    PerThreadErrorKind::Panic
  );
  assert!(handle.join().is_err());
  assert_eq!(
    readiness.workers(),
    [PerThreadWorkerState::Failed, PerThreadWorkerState::Stopped]
  );
}

#[tokio::test]
async fn startup_and_io_failures_reach_every_result_observer() {
  for kind in [PerThreadErrorKind::Startup, PerThreadErrorKind::Io] {
    let mut router = Router::new();
    router.with_state(kind);
    let (workers, shutdown) = spawn_workers(
      "127.0.0.1:0",
      router,
      PerThreadConfig {
        workers: 1,
        ..PerThreadConfig::default()
      },
      #[cfg(not(feature = "compio"))]
      |id, _, router, _, _, _| injected_error(id, router),
      #[cfg(feature = "compio")]
      |id, _, router, _, _| injected_error(id, router),
    )
    .unwrap();
    let handle = PerThreadHandle::new(workers, shutdown);
    let ready = tokio::time::timeout(WORKER_TIMEOUT, handle.wait_ready()).await;

    assert!(ready.unwrap().is_err());
    for _ in 0..2 {
      assert_eq!(handle.result().await.unwrap_err().kind(), kind);
    }

    assert_eq!(handle.join().unwrap_err().kind(), kind);
  }
}

fn injected_error(id: usize, router: std::sync::Arc<Router>) -> Result<(), PerThreadError> {
  let kind = *router.router_state().get::<PerThreadErrorKind>().unwrap();

  Err(PerThreadError::new(
    id,
    kind,
    std::io::Error::other("injected failure"),
  ))
}

#[tokio::test]
async fn unexpected_return_is_a_terminal_failure() {
  let (workers, shutdown) = spawn_workers(
    "127.0.0.1:0",
    Router::new(),
    PerThreadConfig {
      workers: 1,
      ..PerThreadConfig::default()
    },
    #[cfg(not(feature = "compio"))]
    |_, _, _, _, _, _| Ok(()),
    #[cfg(feature = "compio")]
    |_, _, _, _, _| Ok(()),
  )
  .unwrap();
  let handle = PerThreadHandle::new(workers, shutdown);
  let result = tokio::time::timeout(WORKER_TIMEOUT, handle.result()).await;

  assert_eq!(
    result.unwrap().unwrap_err().kind(),
    PerThreadErrorKind::UnexpectedExit
  );
  assert!(handle.join().is_err());
}

#[test]
fn serving_returns_worker_failure_without_waiting_for_an_os_signal() {
  let (workers, shutdown) = spawn_workers(
    "127.0.0.1:0",
    Router::new(),
    PerThreadConfig {
      workers: 1,
      ..PerThreadConfig::default()
    },
    #[cfg(not(feature = "compio"))]
    |_, _, _, _, _, _| panic!("failed server worker"),
    #[cfg(feature = "compio")]
    |_, _, _, _, _| panic!("failed server worker"),
  )
  .unwrap();
  let (sent, received) = std::sync::mpsc::channel();
  let serving = std::thread::spawn(move || {
    sent
      .send(crate::wait_for_shutdown(PerThreadHandle::new(
        workers, shutdown,
      )))
      .unwrap();
  });
  let result = received.recv_timeout(WORKER_TIMEOUT);

  assert!(
    result
      .expect("serving waited for an OS signal after worker failure")
      .is_err()
  );
  serving.join().unwrap();
}

#[tokio::test]
async fn one_failed_bind_does_not_stop_a_listening_worker() {
  let (workers, shutdown) = spawn_workers(
    "127.0.0.1:0",
    Router::new(),
    PerThreadConfig {
      workers: 2,
      ..PerThreadConfig::default()
    },
    #[cfg(not(feature = "compio"))]
    |id, _, _, _, shutdown, _| bind_or_wait(id, shutdown),
    #[cfg(feature = "compio")]
    |id, _, _, _, shutdown| bind_or_wait(id, shutdown),
  )
  .unwrap();
  let handle = PerThreadHandle::new(workers, shutdown);
  let readiness = handle.readiness();
  handle.wait_ready().await.unwrap();
  assert!(readiness.is_ready());

  handle.shutdown();
  assert!(!readiness.is_ready());
  handle.result().await.unwrap();
  handle.join().unwrap();
  assert_eq!(
    readiness.workers(),
    [PerThreadWorkerState::Failed, PerThreadWorkerState::Stopped]
  );
}

fn bind_or_wait(id: usize, shutdown: PerThreadShutdown) -> Result<(), PerThreadError> {
  if id == 0 {
    return Err(PerThreadError::new(
      id,
      PerThreadErrorKind::Bind,
      std::io::Error::new(std::io::ErrorKind::AddrInUse, "occupied listener"),
    ));
  }

  shutdown.report_bind_success(id);
  tokio::runtime::Builder::new_current_thread()
    .build()
    .unwrap()
    .block_on(shutdown.notified());
  Ok(())
}

#[tokio::test]
async fn worker_panic_cancels_its_peers() {
  let (workers, shutdown) = spawn_workers(
    "127.0.0.1:0",
    Router::new(),
    PerThreadConfig {
      workers: 2,
      ..PerThreadConfig::default()
    },
    #[cfg(not(feature = "compio"))]
    |id, _, _, _, shutdown, _| {
      panic_or_wait(id, shutdown);
      Ok(())
    },
    #[cfg(feature = "compio")]
    |id, _, _, _, shutdown| {
      panic_or_wait(id, shutdown);
      Ok(())
    },
  )
  .unwrap();
  let stopped = tokio::time::timeout(WORKER_TIMEOUT, shutdown.notified()).await;
  shutdown.trigger();
  for worker in workers {
    let _ = worker.join();
  }

  assert!(
    stopped.is_ok(),
    "worker panic did not stop surviving workers"
  );
}

fn panic_or_wait(id: usize, shutdown: crate::PerThreadShutdown) {
  assert_ne!(id, 0, "worker failure");
  tokio::runtime::Builder::new_current_thread()
    .build()
    .unwrap()
    .block_on(shutdown.notified());
}

#[tokio::test]
async fn panic_before_bind_wakes_readiness_waiters() {
  let (workers, shutdown) = spawn_workers(
    "127.0.0.1:0",
    Router::new(),
    PerThreadConfig {
      workers: 1,
      ..PerThreadConfig::default()
    },
    #[cfg(not(feature = "compio"))]
    |_, _, _, _, _, _| panic!("startup panic"),
    #[cfg(feature = "compio")]
    |_, _, _, _, _| panic!("startup panic"),
  )
  .unwrap();
  let ready = tokio::time::timeout(WORKER_TIMEOUT, shutdown.wait_for_bind_outcome(1)).await;

  shutdown.trigger();
  for worker in workers {
    assert!(
      worker.join().is_err(),
      "legacy thread joins must preserve worker panics"
    );
  }

  assert!(ready.is_ok(), "worker panic left readiness pending forever");
  assert!(ready.unwrap().is_err());
}
