use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use futures_util::FutureExt;

use super::Job;
use super::QueueError;
#[cfg(feature = "signals")]
use super::emit_queue_signal;
use super::pending::PendingJob;
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

    let notified = inner.notify.notified();
    let mut notified = std::pin::pin!(notified);
    // Register before checking the queue so enqueue/shutdown cannot lose a wake-up.
    notified.as_mut().enable();

    if inner.shutdown.load(Ordering::SeqCst) {
      break;
    }

    let (job, deadline) = {
      let mut pending = inner.pending.lock();
      if inner.shutdown.load(Ordering::SeqCst) {
        continue;
      }
      let job = pending.pop_ready(Instant::now());
      if job.is_some() {
        // Shutdown observes claimed jobs before it can acquire the same pending lock.
        inner.inflight.fetch_add(1, Ordering::SeqCst);
      }
      (job, pending.next_deadline())
    };

    let Some(pending_job) = job else {
      if let Some(deadline) = deadline {
        #[cfg(not(feature = "compio"))]
        let sleep = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline));
        #[cfg(feature = "compio")]
        let sleep = compio::time::sleep_until(deadline);
        let sleep = std::pin::pin!(sleep);
        let _ = futures_util::future::select(notified, sleep).await;
      } else {
        notified.await;
      }
      processed = 0;
      continue;
    };
    let mut inflight = InflightJob {
      inner: &inner,
      job: Some(pending_job),
    };
    let pending_job = inflight.job.as_ref().unwrap();
    processed += 1;

    let handler = inner.handlers.lock().get(&pending_job.name).cloned();

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
      let attempts = pending_job.attempt + 1;
      inner.dead_letter(
        inflight.job.take().unwrap(),
        attempts,
        "no handler registered".into(),
      );
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

    if result.is_ok() {
      #[cfg(feature = "signals")]
      let (name, id, attempt) = {
        let completed = inflight.job.take().unwrap();
        (completed.name, completed.id, completed.attempt)
      };
      #[cfg(not(feature = "signals"))]
      inflight.job.take();
      drop(inflight);
      #[cfg(feature = "signals")]
      emit_queue_signal(signal_ids::QUEUE_JOB_COMPLETED, &name, id, attempt).await;
      continue;
    }

    #[cfg(feature = "signals")]
    emit_queue_signal(
      signal_ids::QUEUE_JOB_FAILED,
      &pending_job.name,
      pending_job.id,
      pending_job.attempt,
    )
    .await;

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

        let Some(run_after) = Instant::now().checked_add(delay) else {
          #[cfg(feature = "signals")]
          emit_queue_signal(
            signal_ids::QUEUE_JOB_DEAD_LETTER,
            &pending_job.name,
            pending_job.id,
            next_attempt,
          )
          .await;
          inner.dead_letter(
            inflight.job.take().unwrap(),
            next_attempt,
            format!("retry delay cannot be represented: {e}"),
          );
          continue;
        };

        let mut job = inflight.job.take().unwrap();
        job.attempt = next_attempt;
        job.run_after = Some(run_after);
        {
          let mut pending = inner.pending.lock();
          if inner.shutdown.load(Ordering::SeqCst) {
            inner.dead_letter(job, next_attempt, "queue shutdown before retry ran".into());
          } else {
            pending.push(job);
          }
        }

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

        let attempts = pending_job.attempt + 1;
        inner.dead_letter(inflight.job.take().unwrap(), attempts, e.to_string());
      }
    }
  }
}

struct InflightJob<'a> {
  inner: &'a QueueInner,
  job: Option<PendingJob>,
}

impl Drop for InflightJob<'_> {
  fn drop(&mut self) {
    if let Some(job) = self.job.take() {
      let attempts = job.attempt + 1;
      self
        .inner
        .dead_letter(job, attempts, "job cancelled before completion".into());
    }
    self.inner.inflight.fetch_sub(1, Ordering::SeqCst);
  }
}
