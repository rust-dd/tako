use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use tako::queue::Queue;
use tako::queue::RetryPolicy;

async fn wait_until(mut condition: impl FnMut() -> bool) {
  let deadline = Instant::now() + Duration::from_secs(1);
  while !condition() {
    assert!(
      Instant::now() < deadline,
      "queue did not finish within one second"
    );
    #[cfg(not(feature = "compio"))]
    tokio::time::sleep(Duration::from_millis(1)).await;
    #[cfg(feature = "compio")]
    compio::time::sleep(Duration::from_millis(1)).await;
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn drains_ready_backlog_without_a_poll_interval_between_jobs() {
  let queue = Queue::builder().workers(4).build();
  let completed = Arc::new(AtomicUsize::new(0));
  let counter = completed.clone();
  queue.register("noop", move |_| {
    let counter = counter.clone();
    async move {
      counter.fetch_add(1, Ordering::SeqCst);
      Ok(())
    }
  });
  for _ in 0..200 {
    queue.push("noop", &()).await.unwrap();
  }
  queue.start();
  wait_until(|| completed.load(Ordering::SeqCst) == 200).await;
  queue.shutdown(Duration::from_secs(1)).await;
  assert_eq!(queue.inflight_count(), 0);
  assert_eq!(queue.dead_letter_count(), 0);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn reregistering_a_job_replaces_the_handler() {
  let queue = Queue::builder().workers(1).build();
  let selected = Arc::new(AtomicUsize::new(0));
  for value in [1, 2] {
    let selected = selected.clone();
    queue.register("replace", move |_| {
      let selected = selected.clone();
      async move {
        selected.store(value, Ordering::SeqCst);
        Ok(())
      }
    });
  }
  queue.push("replace", &()).await.unwrap();
  queue.start();
  wait_until(|| selected.load(Ordering::SeqCst) != 0).await;
  queue.shutdown(Duration::from_secs(1)).await;
  assert_eq!(selected.load(Ordering::SeqCst), 2);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn panicking_handlers_go_to_dead_letters_and_the_worker_survives() {
  let queue = Queue::builder()
    .workers(1)
    .retry(RetryPolicy::fixed(3, Duration::ZERO))
    .build();
  queue.register("poll-panic", |_| async {
    panic!("panic while polling");
  });
  queue.register(
    "call-panic",
    |_| -> std::future::Ready<Result<(), tako::queue::QueueError>> {
      panic!("panic before constructing future");
    },
  );
  let completed = Arc::new(AtomicUsize::new(0));
  let counter = completed.clone();
  queue.register("healthy", move |_| {
    let counter = counter.clone();
    async move {
      counter.fetch_add(1, Ordering::SeqCst);
      Ok(())
    }
  });
  let poll_id = queue.push("poll-panic", &42).await.unwrap();
  let call_id = queue.push("call-panic", &43).await.unwrap();
  queue.push("healthy", &()).await.unwrap();
  queue.start();
  wait_until(|| completed.load(Ordering::SeqCst) == 1).await;
  queue.shutdown(Duration::from_secs(1)).await;
  assert_eq!(queue.inflight_count(), 0);
  assert_eq!(queue.pending_count(), 0);
  let dead = queue.dead_letters();
  assert_eq!(dead.len(), 2);
  assert_eq!(dead[0].id, poll_id);
  assert_eq!(dead[1].id, call_id);
  assert_eq!(dead[0].payload, b"42");
  assert!(
    dead
      .iter()
      .all(|job| job.attempts == 1 && job.error.contains("panicked"))
  );
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn delayed_jobs_do_not_block_ready_jobs_or_run_early() {
  let queue = Queue::builder().workers(1).build();
  let completed = Arc::new(AtomicUsize::new(0));
  let counter = completed.clone();
  queue.register("delayed", move |_| {
    let counter = counter.clone();
    async move {
      counter.fetch_add(1, Ordering::SeqCst);
      Ok(())
    }
  });
  queue
    .push_delayed("delayed", &(), Duration::from_secs(60))
    .await
    .unwrap();
  for _ in 0..100 {
    queue.push("delayed", &()).await.unwrap();
  }
  queue.start();
  wait_until(|| completed.load(Ordering::SeqCst) == 100).await;
  assert_eq!(queue.pending_count(), 1);
  queue.shutdown(Duration::from_secs(1)).await;
  wait_until(|| queue.dead_letter_count() == 1).await;
  assert_eq!(completed.load(Ordering::SeqCst), 100);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn earlier_delayed_jobs_wake_workers_waiting_on_a_later_deadline() {
  let queue = Queue::builder().workers(2).build();
  let completed = Arc::new(AtomicUsize::new(0));
  let counter = completed.clone();
  queue.register("scheduled", move |_| {
    let counter = counter.clone();
    async move {
      counter.fetch_add(1, Ordering::SeqCst);
      Ok(())
    }
  });
  queue
    .push_delayed("scheduled", &(), Duration::from_secs(60))
    .await
    .unwrap();
  queue.start();
  #[cfg(not(feature = "compio"))]
  tokio::time::sleep(Duration::from_millis(5)).await;
  #[cfg(feature = "compio")]
  compio::time::sleep(Duration::from_millis(5)).await;
  let enqueued = Instant::now();
  queue
    .push_delayed("scheduled", &(), Duration::from_millis(10))
    .await
    .unwrap();
  wait_until(|| completed.load(Ordering::SeqCst) == 1).await;
  assert!(enqueued.elapsed() >= Duration::from_millis(10));
  assert_eq!(queue.pending_count(), 1);
  queue.shutdown(Duration::from_secs(1)).await;
  wait_until(|| queue.dead_letter_count() == 1).await;
}
