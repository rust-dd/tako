//! Background worker loop that drains pending jobs and applies retry/DLQ policy.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use futures_util::FutureExt;

use super::DeadJob;
use super::Job;
use super::QueueError;
#[cfg(feature = "signals")]
use super::emit_queue_signal;
use super::runtime::PendingJob;
use super::runtime::QueueInner;
#[cfg(feature = "signals")]
use super::signal_ids;

pub(crate) async fn worker_loop(inner: Arc<QueueInner>) {
  let mut processed = 0;
  loop {
    if processed == 64 {
      // Ready futures need an explicit yield so a busy queue cannot starve I/O or shutdown.
      let mut yielded = false;
      std::future::poll_fn(|cx| {
        if std::mem::replace(&mut yielded, true) {
          std::task::Poll::Ready(())
        } else {
          cx.waker().wake_by_ref();
          std::task::Poll::Pending
        }
      })
      .await;
      processed = 0;
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
      let drained = {
        let mut pending = inner.pending.lock();
        if pending.is_empty() {
          break;
        }
        pending.drain(..).collect::<Vec<_>>()
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

    let job = {
      let mut pending = inner.pending.lock();
      if inner.shutdown.load(Ordering::SeqCst) {
        continue;
      }
      let now = Instant::now();

      let pos = pending.iter().position(|j| match j.run_after {
        Some(t) => now >= t,
        None => true,
      });

      let job = pos.and_then(|i| pending.remove(i));
      if job.is_some() {
        // Shutdown observes claimed jobs before it can acquire the same pending lock.
        inner.inflight.fetch_add(1, Ordering::SeqCst);
      }
      job
    };

    let Some(pending_job) = job else {
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
      processed = 0;
      continue;
    };
    let _inflight = InflightJob(&inner);
    processed += 1;

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

    let (result, panicked) = match AssertUnwindSafe(async { handler(job).await })
      .catch_unwind()
      .await
    {
      Ok(result) => (result, false),
      Err(_) => (
        Err(QueueError::HandlerError("job handler panicked".into())),
        true,
      ),
    };

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
      let max_retries = if panicked {
        0
      } else {
        inner.retry_policy.max_retries()
      };

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
  }
}

struct InflightJob<'a>(&'a QueueInner);

impl Drop for InflightJob<'_> {
  fn drop(&mut self) {
    let prev = self.0.inflight.fetch_sub(1, Ordering::SeqCst);
    if prev == 1 && self.0.shutdown.load(Ordering::SeqCst) {
      self.0.drain_notify.notify_one();
    }
  }
}
