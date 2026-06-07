use std::time::Duration;

/// Configuration for [`serve_per_thread`](crate::serve_per_thread) (and the `compio` variant when enabled).
#[derive(Debug, Clone)]
pub struct PerThreadConfig {
  /// Number of worker threads. Defaults to the number of logical CPUs.
  pub workers: usize,
  /// Pin each worker to a CPU core (requires the `affinity` feature).
  pub pin_to_core: bool,
  /// `SO_REUSEPORT` listen backlog.
  pub backlog: i32,
  /// Maximum time the coordinator waits for in-flight requests after shutdown.
  /// Workers are dropped after this elapses.
  pub drain_timeout: Duration,
}

impl Default for PerThreadConfig {
  fn default() -> Self {
    Self {
      workers: num_cpus(),
      pin_to_core: cfg!(feature = "affinity"),
      backlog: 1024,
      drain_timeout: Duration::from_secs(30),
    }
  }
}

fn num_cpus() -> usize {
  std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
}
