//! Cron scheduling for the queue.
//!
//! Wraps the `cron` crate's `Schedule::upcoming` iterator and exposes a
//! [`CronScheduler`](crate::queue::cron::CronScheduler) that, given a backend,
//! periodically pushes the same payload to a named queue.
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
  ///
  /// Tokio variant — pins to `tokio::time` so `Queue::start` workers on the
  /// tokio runtime can share the same reactor.
  #[cfg(not(feature = "compio"))]
  pub async fn run(self) {
    loop {
      let Some(next) = self.schedule.upcoming(Utc).next() else {
        return;
      };
      let now = Utc::now();
      let wait = (next - now).to_std().unwrap_or(Duration::from_secs(0));
      // Pin the wakeup to a concrete monotonic instant. `tokio::sleep(wait)`
      // accumulates micro-overshoot across iterations because each `wait` is
      // recomputed from the post-sleep wall clock; `sleep_until(deadline)`
      // resolves exactly at `deadline` even after task descheduling.
      let deadline = tokio::time::Instant::now() + wait;
      tokio::time::sleep_until(deadline).await;
      let _ = self
        .backend
        .push(&self.queue, self.payload.as_slice(), PushOptions::default())
        .await;
    }
  }

  /// Drive the scheduler in the current async context until cancelled.
  ///
  /// Compio variant — `Queue::start` spawns onto the compio runtime under the
  /// `compio` feature, so cron must use `compio::time` too. Hard-coding
  /// `tokio::time::*` here used to guarantee a "no reactor running" panic
  /// in any compio + queue-cron deployment without a parallel tokio runtime.
  #[cfg(feature = "compio")]
  pub async fn run(self) {
    loop {
      let Some(next) = self.schedule.upcoming(Utc).next() else {
        return;
      };
      let now = Utc::now();
      let wait = (next - now).to_std().unwrap_or(Duration::from_secs(0));
      // Same monotonic-anchor rationale as the tokio path.
      let deadline = std::time::Instant::now() + wait;
      compio::time::sleep_until(deadline).await;
      let _ = self
        .backend
        .push(&self.queue, self.payload.as_slice(), PushOptions::default())
        .await;
    }
  }
}
