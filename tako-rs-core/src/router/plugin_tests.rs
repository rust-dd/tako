use std::sync::Arc;
use std::sync::Barrier;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::Duration;

use super::Router;
use crate::plugins::TakoPlugin;

#[derive(Clone)]
struct PausedPlugin {
  entered: mpsc::Sender<()>,
  release: Arc<Barrier>,
  calls: Arc<AtomicUsize>,
}

impl TakoPlugin for PausedPlugin {
  fn name(&self) -> &'static str {
    "paused"
  }

  fn setup(&self, router: &Router) -> anyhow::Result<()> {
    self.calls.fetch_add(1, Ordering::SeqCst);
    self.entered.send(()).unwrap();
    self.release.wait();
    router.middleware(|req, next| async move { next.run(req).await });
    Ok(())
  }
}

fn assert_concurrent_setup_waits(
  initialize: impl Fn() + Send + Sync,
  entered: mpsc::Receiver<()>,
  release: &Barrier,
) {
  let initialize = &initialize;
  std::thread::scope(|scope| {
    let first = scope.spawn(initialize);
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    let (calling, called) = mpsc::channel();
    let (finished, completion) = mpsc::channel();
    let second = scope.spawn(move || {
      calling.send(()).unwrap();
      initialize();
      finished.send(()).unwrap();
    });
    called.recv_timeout(Duration::from_secs(2)).unwrap();
    let early_completion = completion.recv_timeout(Duration::from_millis(100));
    release.wait();
    first.join().unwrap();
    second.join().unwrap();
    assert!(matches!(
      early_completion,
      Err(mpsc::RecvTimeoutError::Timeout)
    ));
  });
}

#[test]
fn router_setup_waits_for_middleware_publication() {
  let (entered, entry) = mpsc::channel();
  let release = Arc::new(Barrier::new(2));
  let calls = Arc::new(AtomicUsize::new(0));
  let mut router = Router::new();
  router.plugin(PausedPlugin {
    entered,
    release: release.clone(),
    calls: calls.clone(),
  });
  assert_concurrent_setup_waits(|| router.setup_plugins_once().unwrap(), entry, &release);
  router.setup_plugins_once().unwrap();
  assert_eq!(calls.load(Ordering::SeqCst), 1);
  assert_eq!(router.middlewares.load().len(), 1);
}

#[test]
fn route_setup_waits_for_middleware_publication() {
  let (entered, entry) = mpsc::channel();
  let release = Arc::new(Barrier::new(2));
  let calls = Arc::new(AtomicUsize::new(0));
  let mut router = Router::new();
  let route = router.get("/", || async { "ok" });
  route.middleware(|req, next| async move { next.run(req).await });
  route.plugin(PausedPlugin {
    entered,
    release: release.clone(),
    calls: calls.clone(),
  });
  assert_concurrent_setup_waits(|| route.setup_plugins_once().unwrap(), entry, &release);
  route.setup_plugins_once().unwrap();
  assert_eq!(calls.load(Ordering::SeqCst), 1);
  assert_eq!(route.middlewares.load().len(), 2);
}
