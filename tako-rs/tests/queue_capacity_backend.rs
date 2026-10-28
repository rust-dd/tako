use tako::queue::QueueLimits;
use tako::queue::backend::MemoryBackend;
use tako::queue::backend::PushOptions;
use tako::queue::backend::QueueBackend;

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn retry_at_pending_capacity_keeps_the_reserved_job() {
  let backend = MemoryBackend::new().with_max_pending(1);
  let first = backend
    .push("q", b"first", PushOptions::default())
    .await
    .unwrap();
  backend.reserve("q").await.unwrap().unwrap();
  backend
    .push("q", b"second", PushOptions::default())
    .await
    .unwrap();

  assert!(backend.fail(first, None).await.is_err());
  assert_eq!(backend.pending_count(), 1);
  assert_eq!(backend.reserved_count(), 1);

  let second = backend.reserve("q").await.unwrap().unwrap();
  backend.complete(second.id).await.unwrap();
  backend.fail(first, None).await.unwrap();
  let retry = backend.reserve("q").await.unwrap().unwrap();
  assert_eq!(retry.id, first);
  assert_eq!(retry.payload, b"first");
  assert_eq!(retry.attempt, 1);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn duplicate_push_succeeds_at_pending_capacity() {
  let backend = MemoryBackend::new().with_max_pending(1);
  let opts = PushOptions {
    dedup_key: Some("same".into()),
    ..PushOptions::default()
  };
  backend.push("q", b"first", opts.clone()).await.unwrap();
  assert!(backend.push("q", b"duplicate", opts).await.is_ok());
  assert_eq!(backend.pending_count(), 1);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn reserved_and_retrying_jobs_share_the_backend_budget() {
  let backend =
    MemoryBackend::new().with_limits(QueueLimits::new().max_jobs(1).max_payload_bytes(5));
  let first = backend
    .push("q", b"first", PushOptions::default())
    .await
    .unwrap();
  backend.reserve("q").await.unwrap().unwrap();
  assert!(
    backend
      .try_push("q", b"next", PushOptions::default())
      .await
      .is_err()
  );
  backend.fail(first, None).await.unwrap();
  assert_eq!(backend.outstanding_count(), 1);
  assert_eq!(backend.outstanding_payload_bytes(), 5);
  backend.reserve("q").await.unwrap().unwrap();
  backend.complete(first).await.unwrap();
  assert_eq!(backend.outstanding_count(), 0);
  assert_eq!(backend.outstanding_payload_bytes(), 0);
  backend
    .try_push("q", b"next", PushOptions::default())
    .await
    .unwrap();
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn backend_dead_letters_release_active_budget_and_evict_oldest() {
  let backend = MemoryBackend::new().with_limits(
    QueueLimits::new()
      .max_jobs(1)
      .max_dead_letters(2)
      .max_dead_letter_bytes(3),
  );
  for payload in [b"a".as_slice(), b"bb", b"c"] {
    let id = backend
      .push("q", payload, PushOptions::default())
      .await
      .unwrap();
    backend.reserve("q").await.unwrap().unwrap();
    backend.dead_letter(id).await.unwrap();
    assert_eq!(backend.outstanding_count(), 0);
  }
  let dead = backend.dead_letters();
  assert_eq!(dead.len(), 2);
  assert_eq!(dead[0].2, b"bb");
  assert_eq!(dead[1].2, b"c");
  assert_eq!(backend.dead_letter_payload_bytes(), 3);
  assert_eq!(backend.dead_letter_evictions(), 1);
}
