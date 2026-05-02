//! Cron scheduling for the queue.
//!
//! Wraps the `cron` crate's `Schedule::upcoming` iterator and exposes a
//! [`CronScheduler`] that, given a backend, periodically pushes the same
//! payload to a named queue.
//!
//! ⚠️ Requires the `queue-cron` cargo feature on `tako-core`.

#![cfg(feature = "queue-cron")]

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use cron::Schedule;

use super::backend::PushOptions;
use super::backend::QueueBackend;

/// Periodic-push driver bound to a `QueueBackend`.
pub struct CronScheduler {
  schedule: Schedule,
  queue: String,
  payload: Arc<Vec<u8>>,
  backend: Arc<dyn QueueBackend>,
}

impl CronScheduler {
  /// Build a scheduler from a `"0 9 * * * *"` (sec min hr dom mon dow)-style cron expression.
  pub fn new(
    expression: &str,
    queue: impl Into<String>,
    payload: Vec<u8>,
    backend: Arc<dyn QueueBackend>,
  ) -> Result<Self, cron::error::Error> {
    let schedule = Schedule::from_str(expression)?;
    Ok(Self {
      schedule,
      queue: queue.into(),
      payload: Arc::new(payload),
      backend,
    })
  }

  /// Drive the scheduler in the current async context until cancelled.
  pub async fn run(self) {
    loop {
      let next = match self.schedule.upcoming(Utc).next() {
        Some(t) => t,
        None => return,
      };
      let now = Utc::now();
      let wait = (next - now).to_std().unwrap_or(Duration::from_secs(0));
      tokio::time::sleep(wait).await;
      let _ = self
        .backend
        .push(&self.queue, self.payload.as_slice(), PushOptions::default())
        .await;
    }
  }
}
