//! Background worker loop that drains pending jobs and applies retry/DLQ policy.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use super::DeadJob;
use super::Job;
#[cfg(feature = "signals")]
use super::emit_queue_signal;
use super::runtime::PendingJob;
use super::runtime::QueueInner;
#[cfg(feature = "signals")]
use super::signal_ids;

pub(crate) async fn worker_loop(inner: Arc<QueueInner>) {
  loop {
    // Wait for notification or check periodically for delayed jobs
    #[cfg(not(feature = "compio"))]
    {
      let _ = tokio::time::timeout(Duration::from_millis(100), inner.notify.notified()).await;
    }
    #[cfg(feature = "compio")]
    {
      let notified = std::pin::pin!(inner.notify.notified());
      let sleep = std::pin::pin!(compio::time::sleep(Duration::from_millis(100)));
      let _ = futures_util::future::select(notified, sleep).await;
    }

    if inner.shutdown.load(Ordering::SeqCst) {
      // Drain remaining pending jobs into the DLQ before exiting. Delayed
      // and retry-scheduled jobs sit in `pending` with `run_after > now`;
      // if we exited via `is_empty()` only, the worker would spin until
      // every retry fired (defeating `shutdown(timeout)`) or until the
      // runtime aborted the task — in which case the jobs would silently
      // vanish from memory. Moving them to dead-letters preserves them
      // for `dead_letters()` inspection and any out-of-band re-enqueue
      // after the next startup. The drain happens under the pending lock
      // so concurrent workers see an empty queue and exit cleanly.
      let drained: Vec<PendingJob> = {
        let mut pending = inner.pending.lock();
        if pending.is_empty() {
          break;
        }
        pending.drain(..).collect()
      };
      let mut dlq = inner.dead_letters.lock();
      for pj in drained {
        dlq.push(Arc::new(DeadJob {
          id: pj.id,
          name: pj.name,
          payload: pj.payload,
          attempts: pj.attempt,
          error: "queue shutdown before job ran".into(),
          failed_at: Instant::now(),
        }));
      }
      break;
    }

    // Try to pick up a job
    let job = {
      let mut pending = inner.pending.lock();
      let now = Instant::now();

      // Find the first job that's ready to run
      let pos = pending.iter().position(|j| match j.run_after {
        Some(t) => now >= t,
        None => true,
      });

      pos.and_then(|i| pending.remove(i))
    };

    let Some(pending_job) = job else {
      continue;
    };

    // Look up handler
    let handler = inner
      .handlers
      .get_async(&pending_job.name)
      .await
      .map(|e| e.get().clone());

    let Some(handler) = handler else {
      tracing::warn!("No handler for job '{}', moving to DLQ", pending_job.name);
      #[cfg(feature = "signals")]
      emit_queue_signal(
        signal_ids::QUEUE_JOB_DEAD_LETTER,
        &pending_job.name,
        pending_job.id,
        pending_job.attempt + 1,
      )
      .await;
      inner.dead_letters.lock().push(Arc::new(DeadJob {
        id: pending_job.id,
        name: pending_job.name,
        payload: pending_job.payload,
        attempts: pending_job.attempt + 1,
        error: "no handler registered".into(),
        failed_at: Instant::now(),
      }));
      continue;
    };

    inner.inflight.fetch_add(1, Ordering::SeqCst);

    #[cfg(feature = "signals")]
    emit_queue_signal(
      signal_ids::QUEUE_JOB_STARTED,
      &pending_job.name,
      pending_job.id,
      pending_job.attempt,
    )
    .await;

    let job = Job {
      payload: pending_job.payload.clone(),
      name: pending_job.name.clone(),
      attempt: pending_job.attempt,
      id: pending_job.id,
    };

    let result = handler(job).await;

    #[cfg(feature = "signals")]
    if result.is_ok() {
      emit_queue_signal(
        signal_ids::QUEUE_JOB_COMPLETED,
        &pending_job.name,
        pending_job.id,
        pending_job.attempt,
      )
      .await;
    } else {
      emit_queue_signal(
        signal_ids::QUEUE_JOB_FAILED,
        &pending_job.name,
        pending_job.id,
        pending_job.attempt,
      )
      .await;
    }

    if let Err(e) = result {
      let max_retries = inner.retry_policy.max_retries();

      if pending_job.attempt < max_retries {
        let next_attempt = pending_job.attempt + 1;
        let delay = inner.retry_policy.delay_for_attempt(pending_job.attempt);

        tracing::debug!(
          "Job '{}' (id={}) failed (attempt {}/{}), retrying in {:?}",
          pending_job.name,
          pending_job.id,
          next_attempt,
          max_retries,
          delay
        );

        #[cfg(feature = "signals")]
        emit_queue_signal(
          signal_ids::QUEUE_JOB_RETRYING,
          &pending_job.name,
          pending_job.id,
          next_attempt,
        )
        .await;

        inner.pending.lock().push_back(PendingJob {
          id: pending_job.id,
          name: pending_job.name,
          payload: pending_job.payload,
          attempt: next_attempt,
          run_after: Some(Instant::now() + delay),
          // Preserve the original dedup_key so subsequent `push_dedup`
          // callers continue to see the in-flight retry instead of
          // re-enqueueing a duplicate while the retry sits in `pending`.
          dedup_key: pending_job.dedup_key,
        });

        inner.notify.notify_one();
      } else {
        tracing::warn!(
          "Job '{}' (id={}) exhausted {} retries, moving to DLQ: {}",
          pending_job.name,
          pending_job.id,
          max_retries,
          e
        );

        #[cfg(feature = "signals")]
        emit_queue_signal(
          signal_ids::QUEUE_JOB_DEAD_LETTER,
          &pending_job.name,
          pending_job.id,
          pending_job.attempt + 1,
        )
        .await;

        inner.dead_letters.lock().push(Arc::new(DeadJob {
          id: pending_job.id,
          name: pending_job.name,
          payload: pending_job.payload,
          attempts: pending_job.attempt + 1,
          error: e.to_string(),
          failed_at: Instant::now(),
        }));
      }
    }

    let prev = inner.inflight.fetch_sub(1, Ordering::SeqCst);
    if prev == 1 && inner.shutdown.load(Ordering::SeqCst) {
      inner.drain_notify.notify_one();
    }
  }
}
