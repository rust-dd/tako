#![cfg(feature = "queue-cron")]

#[path = "cron_enqueue/tracing_recorder.rs"]
mod tracing_recorder;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use async_trait::async_trait;
use tako_rs_core::queue::backend::BackendError;
use tako_rs_core::queue::backend::JobId;
use tako_rs_core::queue::backend::MemoryBackend;
use tako_rs_core::queue::backend::PushOptions;
use tako_rs_core::queue::backend::QueueBackend;
use tako_rs_core::queue::backend::ReservedJob;
use tako_rs_core::queue::cron::CronScheduler;
use tracing::instrument::WithSubscriber;
use tracing_recorder::LogRecorder;

async fn sleep(duration: Duration) {
  #[cfg(not(feature = "compio"))]
  tokio::time::sleep(duration).await;
  #[cfg(feature = "compio")]
  compio::time::sleep(duration).await;
}

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
  let work = std::pin::pin!(future);
  let deadline = std::pin::pin!(sleep(Duration::from_secs(2)));
  match futures_util::future::select(work, deadline).await {
    futures_util::future::Either::Left((result, _)) => result,
    futures_util::future::Either::Right(_) => panic!("scheduled occurrence was not observed"),
  }
}

struct Backend {
  memory: MemoryBackend,
  unavailable: AtomicBool,
  attempts: AtomicUsize,
}

#[async_trait]
impl QueueBackend for Backend {
  async fn push(
    &self,
    queue: &str,
    payload: &[u8],
    options: PushOptions,
  ) -> Result<JobId, BackendError> {
    self.attempts.fetch_add(1, Ordering::SeqCst);
    if self.unavailable.load(Ordering::SeqCst) {
      return Err(BackendError::Transport("unavailable".into()));
    }
    self.memory.push(queue, payload, options).await
  }

  async fn reserve(&self, queue: &str) -> Result<Option<ReservedJob>, BackendError> {
    self.memory.reserve(queue).await
  }

  async fn complete(&self, id: JobId) -> Result<(), BackendError> {
    self.memory.complete(id).await
  }

  async fn fail(&self, id: JobId, retry_at: Option<Instant>) -> Result<(), BackendError> {
    self.memory.fail(id, retry_at).await
  }

  async fn dead_letter(&self, id: JobId) -> Result<(), BackendError> {
    self.memory.dead_letter(id).await
  }
}

async fn failed_occurrence_is_observable_and_not_retried(full: bool) {
  let backend = Arc::new(Backend {
    memory: MemoryBackend::new().with_max_pending(1),
    unavailable: AtomicBool::new(!full),
    attempts: AtomicUsize::new(0),
  });
  if full {
    backend
      .memory
      .push("cron", b"existing", PushOptions::default())
      .await
      .unwrap();
  }
  let scheduler = CronScheduler::new(
    "* * * * * *",
    "cron",
    b"scheduled".to_vec(),
    backend.clone(),
  )
  .unwrap();
  let (recorder, mut logs) = LogRecorder::channel();
  let mut run = Box::pin(scheduler.run().with_subscriber(recorder));
  let log = bounded(async {
    tokio::select! {
      _ = &mut run => panic!("scheduler unexpectedly stopped"),
      log = logs.recv() => log.expect("cron must report enqueue failure"),
    }
  })
  .await;
  assert_eq!(log.level, tracing::Level::ERROR);
  assert_eq!(log.fields["queue"], "cron");
  assert!(log.fields.contains_key("scheduled_at"));
  assert_eq!(log.fields["policy"], "drop");
  assert!(log.fields["error"].contains(if full { "queue full" } else { "unavailable" }));

  tokio::select! {
    _ = &mut run => panic!("scheduler unexpectedly stopped"),
    _ = sleep(Duration::from_millis(50)) => {}
  }
  assert_eq!(backend.attempts.load(Ordering::SeqCst), 1);
  assert!(logs.try_recv().is_err());
  if full {
    let existing = backend.reserve("cron").await.unwrap().unwrap();
    assert_eq!(existing.payload, b"existing");
    backend.complete(existing.id).await.unwrap();
  }
  backend.unavailable.store(false, Ordering::SeqCst);
  bounded(async {
    tokio::select! {
      _ = &mut run => panic!("scheduler unexpectedly stopped"),
      _ = async {
        while backend.memory.pending_count() == 0 {
          sleep(Duration::from_millis(5)).await;
        }
      } => {}
    }
  })
  .await;
  assert_eq!(backend.attempts.load(Ordering::SeqCst), 2);
  assert_eq!(backend.memory.pending_count(), 1);
  assert_eq!(
    backend.reserve("cron").await.unwrap().unwrap().payload,
    b"scheduled"
  );
  assert!(logs.try_recv().is_err());
  drop(run);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn full_backend_reports_dropped_occurrence_and_recovers_on_next_tick() {
  failed_occurrence_is_observable_and_not_retried(true).await;
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn transport_failure_reports_dropped_occurrence_and_recovers_on_next_tick() {
  failed_occurrence_is_observable_and_not_retried(false).await;
}
