use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::Serialize;
use serde::Serializer;
use serde::ser::SerializeSeq;
use tako::queue::CapacityExceeded;
use tako::queue::EnqueueError;
use tako::queue::Queue;
use tako::queue::QueueError;
use tako::queue::QueueLimits;

struct LargePayload(Arc<AtomicUsize>);

impl Serialize for LargePayload {
  fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
    let mut seq = serializer.serialize_seq(None)?;
    for _ in 0..100_000 {
      self.0.fetch_add(1, Ordering::SeqCst);
      seq.serialize_element(&1)?;
    }
    seq.end()
  }
}

struct InvalidPayload;

impl Serialize for InvalidPayload {
  fn serialize<S: Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
    Err(serde::ser::Error::custom("invalid payload"))
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn oversized_serialization_stops_before_building_the_whole_payload() {
  let queue = Queue::builder()
    .limits(QueueLimits::new().max_payload_bytes(64))
    .build();
  let serialized = Arc::new(AtomicUsize::new(0));
  let result = queue.try_push("q", &LargePayload(serialized.clone())).await;
  assert!(matches!(
    result,
    Err(EnqueueError::Capacity(CapacityExceeded::PayloadBytes {
      limit: 64
    }))
  ));
  assert!(serialized.load(Ordering::SeqCst) < 1_000);
  assert_eq!(queue.outstanding_count(), 0);
  assert_eq!(queue.outstanding_payload_bytes(), 0);
  queue.try_push("q", &1).await.unwrap();
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn serialization_errors_do_not_consume_capacity() {
  let queue = Queue::builder()
    .limits(QueueLimits::new().max_jobs(1).max_payload_bytes(8))
    .build();
  assert!(matches!(
    queue.try_push("q", &InvalidPayload).await,
    Err(EnqueueError::Queue(QueueError::SerializeError(_)))
  ));
  queue.try_push("q", &1).await.unwrap();
  assert_eq!(queue.outstanding_count(), 1);
  assert_eq!(queue.outstanding_payload_bytes(), 1);
}

#[cfg(not(feature = "compio"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_producers_cannot_oversubscribe_capacity() {
  let queue = Queue::builder()
    .limits(QueueLimits::new().max_jobs(8).max_payload_bytes(8))
    .build();
  let barrier = Arc::new(tokio::sync::Barrier::new(33));
  let mut producers = Vec::new();
  for _ in 0..32 {
    let queue = queue.clone();
    let barrier = barrier.clone();
    producers.push(tokio::spawn(async move {
      barrier.wait().await;
      queue.try_push("q", &1).await
    }));
  }
  barrier.wait().await;
  let mut admitted = 0;
  for producer in producers {
    match producer.await.unwrap() {
      Ok(_) => admitted += 1,
      Err(EnqueueError::Capacity(_)) => {}
      other => panic!("unexpected enqueue result: {other:?}"),
    }
  }
  assert_eq!(admitted, 8);
  assert_eq!(queue.outstanding_count(), 8);
  assert_eq!(queue.outstanding_payload_bytes(), 8);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn cancelled_running_jobs_release_capacity_after_workers_exit() {
  let queue = Queue::builder()
    .workers(1)
    .limits(
      QueueLimits::new()
        .max_jobs(1)
        .max_payload_bytes(1)
        .max_dead_letters(1),
    )
    .build();
  let started = Arc::new(tokio::sync::Notify::new());
  let worker_started = started.clone();
  queue.register("blocked", move |_| {
    let started = worker_started.clone();
    async move {
      started.notify_one();
      std::future::pending().await
    }
  });
  let id = queue.try_push("blocked", &1).await.unwrap();
  let handle = queue.spawn();
  started.notified().await;
  assert_eq!(queue.outstanding_count(), 1);
  assert!(handle.shutdown(Duration::ZERO).await.is_err());
  handle.join().await;
  assert_eq!(queue.outstanding_count(), 0);
  assert_eq!(queue.outstanding_payload_bytes(), 0);
  assert_eq!(queue.dead_letters()[0].id, id);
}
