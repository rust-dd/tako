//! v2 queue regression tests covering dedup keys and the new
//! `MemoryBackend` exposed via the `QueueBackend` trait.

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tako::queue::Queue;
use tako_core::queue::backend::MemoryBackend;
use tako_core::queue::backend::PushOptions;
use tako_core::queue::backend::QueueBackend;

#[tokio::test]
async fn dedup_key_collapses_in_pending() {
  let counter = Arc::new(AtomicU32::new(0));
  let c = counter.clone();

  let queue = Queue::new();
  queue.register("noop", move |_job| {
    let c = c.clone();
    async move {
      c.fetch_add(1, Ordering::SeqCst);
      Ok(())
    }
  });

  // Push three with the same dedup key BEFORE starting workers, so they
  // all sit in pending at once and the dedup check kicks in.
  let id_a = queue.push_dedup("noop", &(), "key-1").await.unwrap();
  let id_b = queue.push_dedup("noop", &(), "key-1").await.unwrap();
  let id_c = queue.push_dedup("noop", &(), "key-1").await.unwrap();
  assert_eq!(id_a, id_b);
  assert_eq!(id_a, id_c);

  queue.start();
  tokio::time::sleep(Duration::from_millis(200)).await;
  assert_eq!(counter.load(Ordering::SeqCst), 1);

  queue.shutdown(Duration::from_secs(1)).await;
}

#[tokio::test]
async fn memory_backend_push_and_reserve_round_trip() {
  let backend = MemoryBackend::new();
  let id = backend
    .push("emails", b"hello", PushOptions::default())
    .await
    .unwrap();

  let job = backend.reserve("emails").await.unwrap().unwrap();
  assert_eq!(job.id, id);
  assert_eq!(job.payload, b"hello");
  backend.complete(job.id).await.unwrap();
  // After completion, queue is empty.
  assert!(backend.reserve("emails").await.unwrap().is_none());
}

#[tokio::test]
async fn memory_backend_dead_letter_path() {
  let backend = MemoryBackend::new();
  let id = backend
    .push("q", b"x", PushOptions::default())
    .await
    .unwrap();
  let _ = backend.reserve("q").await.unwrap().unwrap();
  backend.dead_letter(id).await.unwrap();
  let dlq = backend.dead_letters();
  assert_eq!(dlq.len(), 1);
  assert_eq!(dlq[0].0, id);
}

#[tokio::test]
async fn memory_backend_fail_requeues_with_attempt_increment() {
  let backend = MemoryBackend::new();
  let id = backend
    .push("retryq", b"y", PushOptions::default())
    .await
    .unwrap();
  let job = backend.reserve("retryq").await.unwrap().unwrap();
  assert_eq!(job.attempt, 0);
  backend.fail(id, None).await.unwrap();
  let again = backend.reserve("retryq").await.unwrap().unwrap();
  assert_eq!(again.id, id);
  assert_eq!(again.attempt, 1);
}
