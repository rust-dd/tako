use std::future::Future;
use std::time::Duration;

use tako::queue::Queue;
use tako::queue::QueueError;
use tako::queue::RetryPolicy;
use tokio::sync::mpsc;

async fn bounded<T>(future: impl Future<Output = T>) -> T {
  let future = std::pin::pin!(future);
  let deadline = std::pin::pin!(async {
    #[cfg(not(feature = "compio"))]
    tokio::time::sleep(Duration::from_secs(1)).await;
    #[cfg(feature = "compio")]
    compio::time::sleep(Duration::from_secs(1)).await;
  });
  match futures_util::future::select(future, deadline).await {
    futures_util::future::Either::Left((result, _)) => result,
    futures_util::future::Either::Right(_) => panic!("queue outcome was not recorded"),
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn unrepresentable_retry_deadlines_preserve_jobs_and_keep_the_worker_alive() {
  for retry in [
    RetryPolicy::fixed(1, Duration::MAX),
    RetryPolicy::exponential(1, Duration::MAX),
  ] {
    let queue = Queue::builder().workers(1).retry(retry).build();
    queue.register("overflow", |_| async {
      Err(QueueError::HandlerError("retry requested".into()))
    });
    let (processed, mut completed) = mpsc::unbounded_channel();
    queue.register("healthy", move |_| {
      let processed = processed.clone();
      async move {
        processed.send(()).unwrap();
        Ok(())
      }
    });
    let failed_id = queue.push("overflow", &42).await.unwrap();
    queue.push("healthy", &()).await.unwrap();
    let handle = queue.spawn();

    let healthy = bounded(completed.recv()).await;
    let result = handle.shutdown(Duration::from_secs(1)).await;

    let dead = queue.dead_letters();
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].id, failed_id);
    assert_eq!(dead[0].payload, b"42");
    assert_eq!(dead[0].attempts, 1);
    assert!(dead[0].error.contains("retry delay"));
    assert_eq!(healthy, Some(()));
    assert_eq!(result, Ok(()));
    assert_eq!(queue.pending_count(), 0);
    assert_eq!(queue.inflight_count(), 0);
  }
}

#[cfg(feature = "signals")]
#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn cancelling_a_completion_observer_does_not_dead_letter_a_successful_job() {
  use std::sync::Arc;
  use std::sync::atomic::AtomicUsize;
  use std::sync::atomic::Ordering;

  use tako::queue::QueueRunError;
  use tako::queue::signal_ids;
  use tako::signals::app_signals;

  let (entered, mut observed) = mpsc::unbounded_channel();
  app_signals().on(signal_ids::QUEUE_JOB_COMPLETED, move |signal| {
    let entered = entered.clone();
    async move {
      if signal.metadata.get("name").map(String::as_str) == Some("completed_observer_timeout") {
        entered.send(()).unwrap();
        std::future::pending::<()>().await;
      }
    }
  });
  let queue = Queue::builder().workers(1).build();
  let side_effects = Arc::new(AtomicUsize::new(0));
  let completed = side_effects.clone();
  queue.register("completed_observer_timeout", move |_| {
    let completed = completed.clone();
    async move {
      completed.fetch_add(1, Ordering::SeqCst);
      Ok(())
    }
  });
  queue.push("completed_observer_timeout", &42).await.unwrap();
  let handle = queue.spawn();
  bounded(observed.recv()).await.unwrap();

  assert_eq!(
    handle.shutdown(Duration::ZERO).await,
    Err(QueueRunError::Timeout)
  );
  bounded(handle.join()).await;

  assert_eq!(side_effects.load(Ordering::SeqCst), 1);
  assert_eq!(queue.dead_letter_count(), 0);
  assert_eq!(queue.pending_count(), 0);
  assert_eq!(queue.inflight_count(), 0);
}
