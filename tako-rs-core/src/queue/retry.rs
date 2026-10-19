use std::time::Duration;

/// Retry policy for failed jobs; a panicking handler is never retried.
#[derive(Debug, Clone, Default)]
pub enum RetryPolicy {
  /// Failed jobs go straight to the dead letter queue.
  #[default]
  None,
  Fixed {
    max_retries: u32,
    delay: Duration,
  },
  Exponential {
    max_retries: u32,
    /// Doubled on each retry.
    base_delay: Duration,
  },
}

impl RetryPolicy {
  pub fn fixed(max_retries: u32, delay: Duration) -> Self {
    Self::Fixed { max_retries, delay }
  }

  pub fn exponential(max_retries: u32, base_delay: Duration) -> Self {
    Self::Exponential {
      max_retries,
      base_delay,
    }
  }

  pub(crate) fn max_retries(&self) -> u32 {
    match self {
      Self::None => 0,
      Self::Fixed { max_retries, .. } | Self::Exponential { max_retries, .. } => *max_retries,
    }
  }

  pub(crate) fn delay_for_attempt(&self, attempt: u32) -> Duration {
    match self {
      Self::None => Duration::ZERO,
      Self::Fixed { delay, .. } => *delay,
      Self::Exponential { base_delay, .. } => {
        // A delay that overflows `Duration` waits one day; the job is as good as dead by then.
        base_delay
          .checked_mul(2u32.saturating_pow(attempt))
          .unwrap_or(Duration::from_secs(86_400))
      }
    }
  }
}
