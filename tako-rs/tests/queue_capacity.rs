use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use tako::queue::CapacityExceeded;
use tako::queue::EnqueueError;
use tako::queue::Queue;
use tako::queue::QueueError;
use tako::queue::QueueLimits;
use tako::queue::RetryPolicy;

async fn wait_until(mut ready: impl FnMut() -> bool) {
  let deadline = Instant::now() + Duration::from_secs(2);
  while !ready() {
    assert!(Instant::now() < deadline, "queue did not settle");
    #[cfg(not(feature = "compio"))]
    tokio::time::sleep(Duration::from_millis(1)).await;
    #[cfg(feature = "compio")]
    compio::time::sleep(Duration::from_millis(1)).await;
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn ready_and_delayed_jobs_share_admission_capacity() {
  let queue = Queue::builder()
    .limits(QueueLimits::new().max_jobs(2))
    .build();
  queue
    .try_push_delayed("q", &1, Duration::from_secs(60))
    .await
    .unwrap();
  queue.try_push("q", &2).await.unwrap();
  assert!(matches!(
    queue.try_push("q", &3).await,
    Err(EnqueueError::Capacity(CapacityExceeded::Jobs { limit: 2 }))
  ));
  assert!(queue.push("q", &3).await.is_err());
  assert_eq!(queue.pending_count(), 2);
  assert_eq!(queue.outstanding_count(), 2);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn payload_budget_uses_encoded_bytes_and_rejects_without_admitting() {
  let queue = Queue::builder()
    .limits(QueueLimits::new().max_payload_bytes(5))
    .build();
  queue.try_push("q", &"a").await.unwrap();
  queue.try_push("q", &10).await.unwrap();
  assert_eq!(queue.outstanding_payload_bytes(), 5);
  assert!(matches!(
    queue.try_push("q", &()).await,
    Err(EnqueueError::Capacity(CapacityExceeded::PayloadBytes {
      limit: 5
    }))
  ));
  assert_eq!(queue.pending_count(), 2);
  assert_eq!(queue.outstanding_payload_bytes(), 5);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn dedup_does_not_need_another_slot_or_serialize_an_unused_payload() {
  let queue = Queue::builder()
    .limits(QueueLimits::new().max_jobs(1).max_payload_bytes(1))
    .build();
  let first = queue.try_push_dedup("q", &1, "same").await.unwrap();
  let duplicate = queue
    .try_push_dedup("q", &"too large", "same")
    .await
    .unwrap();
  assert_eq!(duplicate, first);
  assert_eq!(queue.outstanding_count(), 1);
  assert_eq!(queue.outstanding_payload_bytes(), 1);
  assert!(queue.try_push_dedup("q", &2, "other").await.is_err());
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn running_and_retrying_jobs_keep_their_capacity_until_completion() {
  let queue = Queue::builder()
    .workers(1)
    .limits(QueueLimits::new().max_jobs(1).max_payload_bytes(1))
    .retry(RetryPolicy::fixed(1, Duration::ZERO))
    .build();
  let gate = Arc::new(tokio::sync::Semaphore::new(0));
  let worker_gate = gate.clone();
  let (attempt_tx, mut attempts) = tokio::sync::mpsc::unbounded_channel();
  queue.register("q", move |job| {
    let gate = worker_gate.clone();
    let attempts = attempt_tx.clone();
    async move {
      attempts.send(job.attempt).unwrap();
      gate.acquire().await.unwrap().forget();
      if job.attempt == 0 {
        Err(QueueError::HandlerError("retry".into()))
      } else {
        Ok(())
      }
    }
  });
  queue.try_push("q", &1).await.unwrap();
  queue.start();
  for attempt in [0, 1] {
    assert_eq!(attempts.recv().await, Some(attempt));
    assert_eq!(queue.outstanding_count(), 1);
    assert_eq!(queue.outstanding_payload_bytes(), 1);
    assert!(matches!(
      queue.try_push("q", &2).await,
      Err(EnqueueError::Capacity(_))
    ));
    gate.add_permits(1);
  }
  wait_until(|| queue.outstanding_count() == 0).await;
  assert_eq!(queue.outstanding_payload_bytes(), 0);
  queue
    .try_push_delayed("q", &3, Duration::from_secs(60))
    .await
    .unwrap();
  queue.shutdown(Duration::from_secs(1)).await;
  assert_eq!(queue.outstanding_count(), 0);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn dead_letter_retention_keeps_newest_fitting_records_and_counts_evictions() {
  let queue = Queue::builder()
    .workers(1)
    .limits(
      QueueLimits::new()
        .max_dead_letters(2)
        .max_dead_letter_bytes(3),
    )
    .build();
  queue.try_push("missing", &1).await.unwrap();
  let second = queue.try_push("missing", &2).await.unwrap();
  let third = queue.try_push("missing", &3).await.unwrap();
  queue.try_push("missing", &"too large").await.unwrap();
  queue.start();
  wait_until(|| queue.outstanding_count() == 0).await;
  let retained = queue.dead_letters();
  assert_eq!(
    retained.iter().map(|job| job.id).collect::<Vec<_>>(),
    [second, third]
  );
  assert_eq!(queue.dead_letter_payload_bytes(), 2);
  assert_eq!(queue.dead_letter_evictions(), 2);
  queue.clear_dead_letters();
  assert_eq!(queue.dead_letter_payload_bytes(), 0);
  assert_eq!(queue.dead_letter_evictions(), 2);
  queue.shutdown(Duration::from_secs(1)).await;
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn shutdown_releases_capacity_and_applies_dead_letter_retention_before_start() {
  let queue = Queue::builder()
    .limits(QueueLimits::new().max_jobs(2).max_dead_letters(1))
    .build();
  queue.try_push("q", &1).await.unwrap();
  let last = queue
    .try_push_delayed("q", &2, Duration::from_secs(60))
    .await
    .unwrap();
  queue.shutdown(Duration::from_secs(1)).await;
  assert_eq!(queue.outstanding_count(), 0);
  assert_eq!(queue.outstanding_payload_bytes(), 0);
  assert_eq!(queue.dead_letters()[0].id, last);
  assert_eq!(queue.dead_letter_evictions(), 1);
  assert!(matches!(
    queue.try_push("q", &3).await,
    Err(EnqueueError::Queue(QueueError::Shutdown))
  ));
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn zero_capacity_rejects_jobs_and_zero_retention_keeps_no_dead_letters() {
  let closed = Queue::builder()
    .limits(QueueLimits::new().max_jobs(0))
    .build();
  assert!(closed.try_push("q", &()).await.is_err());
  let queue = Queue::builder()
    .limits(QueueLimits::new().max_dead_letters(0))
    .build();
  queue.try_push("missing", &1).await.unwrap();
  queue.start();
  wait_until(|| queue.outstanding_count() == 0).await;
  assert_eq!(queue.dead_letter_count(), 0);
  assert_eq!(queue.dead_letter_evictions(), 1);
  queue.shutdown(Duration::from_secs(1)).await;
}
