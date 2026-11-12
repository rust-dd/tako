#![cfg(all(feature = "per-thread", feature = "plugins", not(feature = "compio")))]

use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::time::Duration;

use tako::PerThreadConfig;
use tako::PerThreadErrorKind;
use tako::plugins::TakoPlugin;
use tako::router::Router;
use tako::spawn_per_thread_supervised;

// Cleanup stays gated; the timeout only bounds hangs and allows for slow panic diagnostics.
const FAILURE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Default)]
struct BlockingCleanup(Arc<(Mutex<bool>, Condvar)>);

impl BlockingCleanup {
  fn wait(&self) {
    let (released, wake) = self.0.as_ref();
    let _guard = wake
      .wait_while(released.lock().unwrap(), |released| !*released)
      .unwrap();
  }

  fn release(&self) {
    let (released, wake) = self.0.as_ref();
    *released.lock().unwrap() = true;
    wake.notify_all();
  }
}

#[derive(Clone)]
struct FailingPlugin {
  cleanup: BlockingCleanup,
  panic: bool,
}

impl TakoPlugin for FailingPlugin {
  fn name(&self) -> &'static str {
    "blocking-startup-failure"
  }

  fn setup(&self, _: &Router) -> anyhow::Result<()> {
    let cleanup = self.cleanup.clone();
    let (started, ready) = std::sync::mpsc::channel();
    let blocking = move || {
      started.send(()).unwrap();
      cleanup.wait();
    };
    tokio::task::spawn_blocking(blocking);
    ready.recv().unwrap();
    assert!(!self.panic, "plugin startup panic");
    Err(std::io::Error::other("plugin startup failed").into())
  }
}

async fn assert_failure_precedes_cleanup(panic: bool, kind: PerThreadErrorKind) {
  let cleanup = BlockingCleanup::default();
  let mut router = Router::new();
  router.plugin(FailingPlugin {
    cleanup: cleanup.clone(),
    panic,
  });
  let server = spawn_per_thread_supervised(
    "127.0.0.1:0",
    router,
    PerThreadConfig {
      workers: 1,
      ..PerThreadConfig::default()
    },
  )
  .unwrap();
  let shutdown = server.shutdown_handle();
  let (ready, result, stopped) = tokio::join!(
    tokio::time::timeout(FAILURE_TIMEOUT, server.wait_ready()),
    tokio::time::timeout(FAILURE_TIMEOUT, server.result()),
    tokio::time::timeout(FAILURE_TIMEOUT, shutdown.notified()),
  );
  cleanup.release();
  let joined = server.join();

  assert!(
    ready
      .expect("readiness waited for runtime cleanup")
      .is_err()
  );
  assert_eq!(
    result
      .expect("failure waited for runtime cleanup")
      .unwrap_err()
      .kind(),
    kind
  );
  stopped.expect("peer cancellation waited for runtime cleanup");
  assert_eq!(joined.unwrap_err().kind(), kind);
}

#[tokio::test]
async fn startup_error_is_reported_before_runtime_waits_for_blocking_tasks() {
  assert_failure_precedes_cleanup(false, PerThreadErrorKind::Startup).await;
}

#[tokio::test]
async fn startup_panic_is_reported_before_runtime_waits_for_blocking_tasks() {
  assert_failure_precedes_cleanup(true, PerThreadErrorKind::Panic).await;
}
