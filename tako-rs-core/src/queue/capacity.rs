use std::sync::Arc;

use parking_lot::Mutex;

use super::CapacityExceeded;
use super::QueueLimits;

#[derive(Default)]
struct Usage {
  limits: QueueLimits,
  jobs: usize,
  bytes: usize,
}

#[derive(Default)]
pub(crate) struct Budget(Mutex<Usage>);

impl Budget {
  pub(crate) fn new(limits: QueueLimits) -> Self {
    Self(Mutex::new(Usage {
      limits,
      ..Usage::default()
    }))
  }

  pub(crate) fn set_limits(&self, limits: QueueLimits) {
    self.0.lock().limits = limits;
  }

  pub(crate) fn limits(&self) -> QueueLimits {
    self.0.lock().limits
  }

  pub(crate) fn usage(&self) -> (usize, usize) {
    let usage = self.0.lock();
    (usage.jobs, usage.bytes)
  }

  pub(crate) fn reserve(self: &Arc<Self>, bytes: usize) -> Result<Permit, CapacityExceeded> {
    let mut usage = self.0.lock();
    let job_limit = usage.limits.jobs.unwrap_or(usize::MAX);
    if usage.jobs >= job_limit {
      return Err(CapacityExceeded::Jobs { limit: job_limit });
    }
    let byte_limit = usage.limits.payload_bytes.unwrap_or(usize::MAX);
    if usage.bytes > byte_limit || bytes > byte_limit - usage.bytes {
      return Err(CapacityExceeded::PayloadBytes { limit: byte_limit });
    }
    usage.jobs += 1;
    usage.bytes += bytes;
    Ok(Permit {
      budget: self.clone(),
      bytes,
    })
  }
}

pub(crate) struct Permit {
  budget: Arc<Budget>,
  bytes: usize,
}

impl Drop for Permit {
  fn drop(&mut self) {
    let mut usage = self.budget.0.lock();
    usage.jobs -= 1;
    usage.bytes -= self.bytes;
  }
}
