#![cfg(feature = "signals")]

use std::time::Duration;

use tako_rs_core::signals::Signal;
use tako_rs_core::signals::SignalArbiter;
use tokio::sync::oneshot;

struct CapturedResource(Option<oneshot::Sender<()>>);

impl Drop for CapturedResource {
  fn drop(&mut self) {
    if let Some(released) = self.0.take() {
      let _ = released.send(());
    }
  }
}

async fn assert_filter_released(emit_rejected_event: bool) {
  let arbiter = SignalArbiter::new();
  let (released, finished) = oneshot::channel();
  let resource = CapturedResource(Some(released));
  let receiver = arbiter.subscribe_filtered("updates", move |_| {
    std::hint::black_box(&resource);
    false
  });
  if emit_rejected_event {
    arbiter.emit(Signal::new("updates")).await;
  }
  drop(receiver);
  let finished = std::pin::pin!(finished);
  let deadline = std::pin::pin!(async {
    #[cfg(not(feature = "compio"))]
    tokio::time::sleep(Duration::from_millis(200)).await;
    #[cfg(feature = "compio")]
    compio::time::sleep(Duration::from_millis(200)).await;
  });
  match futures_util::future::select(finished, deadline).await {
    futures_util::future::Either::Left((result, _)) => result.unwrap(),
    futures_util::future::Either::Right(_) => panic!("dropped receiver retained its filter task"),
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn dropping_a_quiet_filtered_receiver_releases_its_captures() {
  assert_filter_released(false).await;
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn dropping_a_receiver_with_an_always_false_filter_releases_its_captures() {
  assert_filter_released(true).await;
}

#[test]
fn dropped_prefix_receivers_disappear_from_subscription_introspection() {
  let arbiter = SignalArbiter::new();
  let live = arbiter.subscribe_prefix("live.*");
  for _ in 0..100 {
    drop(arbiter.subscribe_prefix("transient.*"));
  }
  assert_eq!(arbiter.signal_prefixes(), ["live.*"]);
  drop(live);
  assert!(arbiter.signal_prefixes().is_empty());
}
