//! Queue signal ids and emission helper.

#![cfg(feature = "signals")]

use crate::signals::Signal;
use crate::signals::SignalArbiter;

/// Well-known queue signal ids.
pub mod signal_ids {
  pub const QUEUE_JOB_QUEUED: &str = "queue.job.queued";
  pub const QUEUE_JOB_STARTED: &str = "queue.job.started";
  pub const QUEUE_JOB_COMPLETED: &str = "queue.job.completed";
  pub const QUEUE_JOB_FAILED: &str = "queue.job.failed";
  pub const QUEUE_JOB_RETRYING: &str = "queue.job.retrying";
  pub const QUEUE_JOB_DEAD_LETTER: &str = "queue.job.dead_letter";
}

pub(crate) async fn emit_queue_signal(id: &'static str, name: &str, job_id: u64, attempt: u32) {
  SignalArbiter::emit_app(
    Signal::with_capacity(id, 3)
      .meta("name", name)
      .meta("id", job_id.to_string())
      .meta("attempt", attempt.to_string()),
  )
  .await;
}
