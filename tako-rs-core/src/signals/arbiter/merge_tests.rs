use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use super::SignalArbiter;
use crate::router::Router;
use crate::signals::Signal;

#[tokio::test]
async fn merging_keeps_both_existing_exact_subscriptions() {
  let parent = SignalArbiter::new();
  let child = SignalArbiter::new();
  let mut parent_rx = parent.subscribe("updates");
  let mut child_rx = child.subscribe("updates");
  parent.merge_from(&child);
  parent.emit(Signal::new("updates")).await;
  assert_eq!(parent_rx.try_recv().unwrap().id.as_ref(), "updates");
  assert_eq!(child_rx.try_recv().unwrap().id.as_ref(), "updates");
  assert!(parent_rx.try_recv().is_err());
  assert!(child_rx.try_recv().is_err());
}

#[tokio::test]
async fn repeated_merges_preserve_each_subscription_and_handler_once() {
  let parent = SignalArbiter::new();
  let child = SignalArbiter::new();
  let mut prefix_rx = child.subscribe_prefix("updates");
  let mut exact_rx = child.subscribe("updates");
  let calls = Arc::new(AtomicUsize::new(0));
  let observed = calls.clone();
  child.on("updates", move |_| {
    observed.fetch_add(1, Ordering::Relaxed);
    async {}
  });
  parent.merge_from(&child);
  parent.merge_from(&child);
  parent.emit(Signal::new("updates")).await;
  assert_eq!(calls.load(Ordering::Relaxed), 1);
  assert!(prefix_rx.try_recv().is_ok());
  assert!(prefix_rx.try_recv().is_err());
  assert!(exact_rx.try_recv().is_ok());
  assert!(exact_rx.try_recv().is_err());
}

#[tokio::test]
async fn nested_router_merges_keep_subscriptions_through_plugin_setup() {
  let mut parent = Router::new();
  let mut middle = Router::new();
  let mut child = Router::new();
  child.get("/", || async { "ok" });
  let mut receivers = Vec::new();
  for router in [&parent, &middle, &child] {
    receivers.push(router.signals().subscribe("updates"));
    receivers.push(router.signals().subscribe_prefix("updates"));
  }
  #[cfg(feature = "plugins")]
  let plugin_rx = {
    let receiver = Arc::new(std::sync::Mutex::new(None));
    child.plugin(SubscribePlugin(receiver.clone()));
    receiver
  };
  middle.merge(child);
  parent.nest("/api", middle);

  for _ in 0..2 {
    let request = http::Request::builder()
      .uri("/api")
      .body(crate::body::TakoBody::empty())
      .unwrap();
    assert_eq!(parent.dispatch(request).await.status(), 200);
    parent.emit_signal(Signal::new("updates")).await;
    for receiver in &mut receivers {
      assert_eq!(receiver.try_recv().unwrap().id.as_ref(), "updates");
      assert!(receiver.try_recv().is_err());
    }
    #[cfg(feature = "plugins")]
    {
      let mut guard = plugin_rx.lock().unwrap();
      let receiver = guard.as_mut().unwrap();
      assert!(receiver.try_recv().is_ok());
      assert!(receiver.try_recv().is_err());
    }
  }
}

#[cfg(feature = "plugins")]
#[derive(Clone)]
struct SubscribePlugin(Arc<std::sync::Mutex<Option<tokio::sync::broadcast::Receiver<Signal>>>>);

#[cfg(feature = "plugins")]
impl crate::plugins::TakoPlugin for SubscribePlugin {
  fn name(&self) -> &'static str {
    "subscribe"
  }

  fn setup(&self, router: &Router) -> anyhow::Result<()> {
    *self.0.lock().unwrap() = Some(router.signals().subscribe("updates"));
    Ok(())
  }
}
