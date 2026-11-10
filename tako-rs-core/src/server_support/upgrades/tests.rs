use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use futures_util::FutureExt;

use super::UpgradeSupervisor;

struct Dropped(Arc<AtomicBool>);

impl Drop for Dropped {
  fn drop(&mut self) {
    self.0.store(true, Ordering::SeqCst);
  }
}

#[tokio::test]
async fn abort_waits_for_session_resources_to_be_dropped() {
  let supervisor = UpgradeSupervisor::default();
  let registry = supervisor.registry();
  let lease = registry.register().unwrap();
  let dropped = Arc::new(AtomicBool::new(false));
  let guard = Dropped(dropped.clone());
  let (started, ready) = tokio::sync::oneshot::channel();
  let task = tokio::spawn(lease.run(async move {
    let _guard = guard;
    started.send(()).unwrap();
    std::future::pending::<()>().await;
  }));
  ready.await.unwrap();

  supervisor.trigger();
  assert!(registry.register().is_none());
  assert!(supervisor.join().now_or_never().is_none());

  supervisor.abort();
  supervisor.join().await;
  assert!(dropped.load(Ordering::SeqCst));
  task.await.unwrap();
}

#[tokio::test]
async fn dropping_owner_cancels_unpolled_and_running_sessions() {
  let supervisor = UpgradeSupervisor::default();
  let registry = supervisor.registry();
  let unpolled = registry.register().unwrap();
  let running = registry.register().unwrap();
  let task = tokio::spawn(running.run(std::future::pending()));

  drop(supervisor);
  unpolled.run(std::future::pending()).await;
  registry.join().await;
  task.await.unwrap();
  assert!(registry.register().is_none());
}

#[tokio::test]
async fn a_session_panic_does_not_leave_shutdown_waiting() {
  let supervisor = UpgradeSupervisor::default();
  let lease = supervisor.registry().register().unwrap();
  let task = tokio::spawn(lease.run(async { panic!("session failed") }));

  assert!(task.await.unwrap_err().is_panic());
  supervisor.trigger();
  supervisor.join().await;
}
