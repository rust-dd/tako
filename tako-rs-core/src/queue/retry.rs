//! Retry policy for failed jobs.

use std::time::Duration;

/// Retry policy for failed jobs.
#[derive(Debug, Clone, Default)]
pub enum RetryPolicy {
  /// No retries — failed jobs go straight to the dead letter queue.
  #[default]
  None,
  /// Fixed delay between retries.
  Fixed {
    /// Maximum number of retry attempts.
    max_retries: u32,
    /// Delay between each retry.
    delay: Duration,
  },
  /// Exponential backoff between retries.
  Exponential {
    /// Maximum number of retry attempts.
    max_retries: u32,
    /// Initial delay (doubled on each retry).
    base_delay: Duration,
  },
}

impl RetryPolicy {
  /// Create a fixed-delay retry policy.
  pub fn fixed(max_retries: u32, delay: Duration) -> Self {
    Self::Fixed { max_retries, delay }
  }

  /// Create an exponential-backoff retry policy.
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
        // `Duration * u32` panics on overflow; `base_delay = 1s, attempt = 64`
        // wraps 2^64 nanos into `u128 * u128` overflow in `Duration::Mul`.
        // Fall back to a 1-day ceiling — any retry waiting longer than that
        // is effectively a dead job; the queue's DLQ pathway should kick in.
        base_delay
          .checked_mul(2u32.saturating_pow(attempt))
          .unwrap_or(Duration::from_secs(86_400))
      }
    }
  }
}
