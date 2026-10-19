//! Cron scheduling for [`QueueBackend`] queues.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use cron::Schedule;

use super::backend::PushOptions;
use super::backend::QueueBackend;

/// Pushes the same payload to one queue each time a cron schedule fires.
pub struct CronScheduler {
  schedule: Schedule,
  queue: String,
  payload: Arc<Vec<u8>>,
  backend: Arc<dyn QueueBackend>,
}

impl CronScheduler {
  /// Parses `expression` in `sec min hour day-of-month month day-of-week` order, e.g.
  /// `"0 9 * * * *"`.
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

  /// Pushes at every fire time until the schedule runs out; drop the future to stop it.
  pub async fn run(self) {
    loop {
      let Some(next) = self.schedule.upcoming(Utc).next() else {
        return;
      };
      let wait = (next - Utc::now()).to_std().unwrap_or(Duration::ZERO);
      #[cfg(not(feature = "compio"))]
      tokio::time::sleep(wait).await;
      #[cfg(feature = "compio")]
      compio::time::sleep(wait).await;
      let _ = self
        .backend
        .push(&self.queue, self.payload.as_slice(), PushOptions::default())
        .await;
    }
  }
}
