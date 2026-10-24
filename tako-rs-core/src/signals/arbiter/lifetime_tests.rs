use super::SignalArbiter;
use crate::signals::Signal;

#[tokio::test]
async fn prefix_churn_does_not_retain_historical_channels() {
  let arbiter = SignalArbiter::new();
  for index in 0..1000 {
    drop(arbiter.subscribe_prefix(format!("transient.{index}")));
  }
  assert!(arbiter.inner.prefix_topics.load().len() <= 1);
  arbiter.emit(Signal::new("unrelated")).await;
  assert!(arbiter.inner.prefix_topics.load().is_empty());
}

#[tokio::test]
async fn cleanup_preserves_live_prefix_subscribers_after_merging() {
  let parent = SignalArbiter::new();
  let child = SignalArbiter::new();
  let mut live = child.subscribe_prefix("updates.*");
  drop(child.subscribe_prefix("expired.*"));
  parent.merge_from(&child);
  assert_eq!(parent.inner.prefix_topics.load().len(), 1);
  parent.emit(Signal::new("updates.changed")).await;
  assert!(live.try_recv().is_ok());
  assert!(live.try_recv().is_err());
  drop(live);
  parent.emit(Signal::new("updates.changed")).await;
  child.emit(Signal::new("updates.changed")).await;
  assert!(parent.inner.prefix_topics.load().is_empty());
  assert!(child.inner.prefix_topics.load().is_empty());
}
