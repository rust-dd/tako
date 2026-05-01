use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tako::queue::Job;
use tako::queue::Queue;
use tako::queue::QueueError;
use tako::queue::RetryPolicy;

#[tokio::test]
async fn push_and_process_job() {
  let counter = Arc::new(AtomicU32::new(0));
  let c = counter.clone();

  let queue = Queue::new();
  queue.register("inc", move |_job: Job| {
    let c = c.clone();
    async move {
      c.fetch_add(1, Ordering::SeqCst);
      Ok(())
    }
  });
  queue.start();

  queue.push("inc", &()).await.unwrap();
  queue.push("inc", &()).await.unwrap();
  queue.push("inc", &()).await.unwrap();

  tokio::time::sleep(Duration::from_millis(300)).await;
  assert_eq!(counter.load(Ordering::SeqCst), 3);

  queue.shutdown(Duration::from_secs(1)).await;
}

#[tokio::test]
async fn job_deserializes_payload() {
  let received = Arc::new(tokio::sync::Mutex::new(String::new()));
  let r = received.clone();

  let queue = Queue::new();
  queue.register("greet", move |job: Job| {
    let r = r.clone();
    async move {
      let name: String = job.deserialize()?;
      *r.lock().await = name;
      Ok(())
    }
  });
  queue.start();

  queue.push("greet", &"hello world").await.unwrap();
  tokio::time::sleep(Duration::from_millis(300)).await;

  assert_eq!(*received.lock().await, "hello world");
  queue.shutdown(Duration::from_secs(1)).await;
}

#[tokio::test]
async fn job_returns_unique_id() {
  let queue = Queue::new();
  queue.register("noop", |_: Job| async { Ok(()) });
  queue.start();

  let id1 = queue.push("noop", &1).await.unwrap();
  let id2 = queue.push("noop", &2).await.unwrap();
  let id3 = queue.push("noop", &3).await.unwrap();

  assert_ne!(id1, id2);
  assert_ne!(id2, id3);
  assert!(id2 > id1);
  assert!(id3 > id2);

  queue.shutdown(Duration::from_secs(1)).await;
}

#[tokio::test]
async fn unknown_job_goes_to_dlq() {
  let queue = Queue::new();
  // No handler registered
  queue.start();

  queue.push("nonexistent", &42).await.unwrap();
  tokio::time::sleep(Duration::from_millis(300)).await;

  let dlq = queue.dead_letters();
  assert_eq!(dlq.len(), 1);
  assert_eq!(dlq[0].name, "nonexistent");
  assert_eq!(dlq[0].error, "no handler registered");

  queue.shutdown(Duration::from_secs(1)).await;
}

#[tokio::test]
async fn failed_job_no_retry_goes_to_dlq() {
  let queue = Queue::new(); // default: no retries

  queue.register("fail", |_: Job| async {
    Err(QueueError::HandlerError("boom".into()))
  });
  queue.start();

  queue.push("fail", &"data").await.unwrap();
  tokio::time::sleep(Duration::from_millis(300)).await;

  let dlq = queue.dead_letters();
  assert_eq!(dlq.len(), 1);
  assert_eq!(dlq[0].name, "fail");
  assert!(dlq[0].error.contains("boom"));
  assert_eq!(dlq[0].attempts, 1);

  queue.shutdown(Duration::from_secs(1)).await;
}

#[tokio::test]
async fn retry_policy_fixed() {
  let attempts = Arc::new(AtomicU32::new(0));
  let a = attempts.clone();

  let queue = Queue::builder()
    .workers(1)
    .retry(RetryPolicy::fixed(2, Duration::from_millis(50)))
    .build();

  queue.register("flaky", move |_: Job| {
    let a = a.clone();
    async move {
      let n = a.fetch_add(1, Ordering::SeqCst);
      if n < 2 {
        Err(QueueError::HandlerError("not yet".into()))
      } else {
        Ok(()) // Succeeds on 3rd attempt (attempt index 2)
      }
    }
  });
  queue.start();

  queue.push("flaky", &()).await.unwrap();
  tokio::time::sleep(Duration::from_millis(500)).await;

  // 3 total attempts: 0, 1 (retry), 2 (retry, succeeds)
  assert_eq!(attempts.load(Ordering::SeqCst), 3);
  // Should NOT be in DLQ since it eventually succeeded
  assert!(queue.dead_letters().is_empty());

  queue.shutdown(Duration::from_secs(1)).await;
}

#[tokio::test]
async fn retry_policy_exhausted_goes_to_dlq() {
  let attempts = Arc::new(AtomicU32::new(0));
  let a = attempts.clone();

  let queue = Queue::builder()
    .workers(1)
    .retry(RetryPolicy::fixed(2, Duration::from_millis(50)))
    .build();

  queue.register("always_fail", move |_: Job| {
    let a = a.clone();
    async move {
      a.fetch_add(1, Ordering::SeqCst);
      Err(QueueError::HandlerError("permanent failure".into()))
    }
  });
  queue.start();

  queue.push("always_fail", &()).await.unwrap();
  tokio::time::sleep(Duration::from_millis(500)).await;

  // 3 total attempts: initial + 2 retries
  assert_eq!(attempts.load(Ordering::SeqCst), 3);
  let dlq = queue.dead_letters();
  assert_eq!(dlq.len(), 1);
  assert_eq!(dlq[0].attempts, 3);

  queue.shutdown(Duration::from_secs(1)).await;
}

#[tokio::test]
async fn delayed_job() {
  let executed_at = Arc::new(tokio::sync::Mutex::new(None));
  let e = executed_at.clone();
  let pushed_at = std::time::Instant::now();

  let queue = Queue::builder().workers(1).build();
  queue.register("delayed", move |_: Job| {
    let e = e.clone();
    async move {
      *e.lock().await = Some(std::time::Instant::now());
      Ok(())
    }
  });
  queue.start();

  queue
    .push_delayed("delayed", &(), Duration::from_millis(200))
    .await
    .unwrap();

  // Should NOT have executed yet
  tokio::time::sleep(Duration::from_millis(50)).await;
  assert!(executed_at.lock().await.is_none());

  // Wait for delay to pass
  tokio::time::sleep(Duration::from_millis(300)).await;
  let at = executed_at.lock().await.unwrap();
  assert!(at.duration_since(pushed_at) >= Duration::from_millis(200));

  queue.shutdown(Duration::from_secs(1)).await;
}

#[tokio::test]
async fn pending_and_inflight_count() {
  let barrier = Arc::new(tokio::sync::Barrier::new(2));
  let b = barrier.clone();

  let queue = Queue::builder().workers(1).build();
  queue.register("slow", move |_: Job| {
    let b = b.clone();
    async move {
      b.wait().await;
      Ok(())
    }
  });
  queue.start();

  queue.push("slow", &()).await.unwrap();
  tokio::time::sleep(Duration::from_millis(50)).await;

  // Job should be inflight now
  assert_eq!(queue.inflight_count(), 1);

  // Release the job
  barrier.wait().await;
  tokio::time::sleep(Duration::from_millis(50)).await;

  assert_eq!(queue.inflight_count(), 0);
  queue.shutdown(Duration::from_secs(1)).await;
}

#[tokio::test]
async fn clear_dead_letters() {
  let queue = Queue::new();
  queue.register("fail", |_: Job| async {
    Err(QueueError::HandlerError("err".into()))
  });
  queue.start();

  queue.push("fail", &()).await.unwrap();
  tokio::time::sleep(Duration::from_millis(200)).await;

  assert_eq!(queue.dead_letters().len(), 1);
  queue.clear_dead_letters();
  assert!(queue.dead_letters().is_empty());

  queue.shutdown(Duration::from_secs(1)).await;
}

#[tokio::test]
async fn shutdown_rejects_new_jobs() {
  let queue = Queue::new();
  queue.register("noop", |_: Job| async { Ok(()) });
  queue.start();

  queue.shutdown(Duration::from_secs(1)).await;

  let result = queue.push("noop", &()).await;
  assert!(result.is_err());
  assert!(matches!(result.unwrap_err(), QueueError::Shutdown));
}

#[tokio::test]
async fn multiple_workers_process_concurrently() {
  let counter = Arc::new(AtomicU32::new(0));
  let max_concurrent = Arc::new(AtomicU32::new(0));
  let c = counter.clone();
  let m = max_concurrent.clone();

  let queue = Queue::builder().workers(4).build();
  queue.register("concurrent", move |_: Job| {
    let c = c.clone();
    let m = m.clone();
    async move {
      let current = c.fetch_add(1, Ordering::SeqCst) + 1;
      // Track max concurrency
      m.fetch_max(current, Ordering::SeqCst);
      tokio::time::sleep(Duration::from_millis(100)).await;
      c.fetch_sub(1, Ordering::SeqCst);
      Ok(())
    }
  });
  queue.start();

  // Push 8 jobs
  for _ in 0..8 {
    queue.push("concurrent", &()).await.unwrap();
  }

  tokio::time::sleep(Duration::from_millis(500)).await;

  // With 4 workers, we should see >1 concurrent execution
  assert!(max_concurrent.load(Ordering::SeqCst) > 1);

  queue.shutdown(Duration::from_secs(1)).await;
}

#[tokio::test]
async fn exponential_backoff_retry() {
  let attempts = Arc::new(AtomicU32::new(0));
  let a = attempts.clone();

  let queue = Queue::builder()
    .workers(1)
    .retry(RetryPolicy::exponential(2, Duration::from_millis(50)))
    .build();

  queue.register("exp_fail", move |_: Job| {
    let a = a.clone();
    async move {
      a.fetch_add(1, Ordering::SeqCst);
      Err(QueueError::HandlerError("fail".into()))
    }
  });
  queue.start();

  queue.push("exp_fail", &()).await.unwrap();
  // Wait enough for initial + 50ms retry + 100ms retry
  tokio::time::sleep(Duration::from_millis(600)).await;

  assert_eq!(attempts.load(Ordering::SeqCst), 3); // initial + 2 retries
  assert_eq!(queue.dead_letters().len(), 1);

  queue.shutdown(Duration::from_secs(1)).await;
}
