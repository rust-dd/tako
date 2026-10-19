#![cfg(feature = "signals")]

use tako::signals::Signal;
use tako::signals::SignalArbiter;

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn filtered_subscription_survives_broadcast_lag() {
  let arbiter = SignalArbiter::new();
  let mut stream =
    arbiter.subscribe_filtered("updates", |signal| signal.metadata.contains_key("accepted"));
  for _ in 0..SignalArbiter::global_broadcast_capacity() * 2 {
    arbiter.emit(Signal::new("updates")).await;
  }
  arbiter
    .emit(Signal::new("updates").meta("accepted", "buffered"))
    .await;

  let buffered = stream.recv().await.expect("lag must not close the stream");
  assert_eq!(buffered.metadata["accepted"], "buffered");

  arbiter
    .emit(Signal::new("updates").meta("accepted", "later"))
    .await;
  let later = stream.recv().await.expect("future signals must arrive");
  assert_eq!(later.metadata["accepted"], "later");

  drop(arbiter);
  assert!(stream.recv().await.is_none());
}
