use std::time::Duration;

/// Configuration for [`serve_per_thread`](crate::serve_per_thread) and
/// [`spawn_per_thread`](crate::spawn_per_thread).
#[derive(Debug, Clone)]
pub struct PerThreadConfig {
  /// Number of worker threads. Defaults to the number of logical CPUs.
  pub workers: usize,
  /// Pins worker N to core N (`affinity` feature). Off by default, since a pinned worker cannot
  /// leave a core another pinning process keeps busy; use it when the server owns the machine.
  pub pin_to_core: bool,
  /// `SO_REUSEPORT` listen backlog.
  pub backlog: i32,
  /// How long each worker waits for in-flight connections after shutdown before dropping them.
  pub drain_timeout: Duration,
  /// How long a connection may go without a complete request head; `None` disables it. Checked
  /// every half deadline, so an idle connection closes after 1 to 1.5 times this.
  pub header_read_timeout: Option<Duration>,
  /// Concurrent connections per worker, `None` for no cap; an upgraded connection such as a
  /// WebSocket counts until its socket closes.
  pub max_connections: Option<usize>,
  /// Tokio workers hand a connection to the least busy worker when they are ahead, evening out
  /// `SO_REUSEPORT`, which on some kernels sends every connection to one listener. On by default.
  pub balance_connections: bool,
}

impl Default for PerThreadConfig {
  fn default() -> Self {
    Self {
      workers: num_cpus(),
      pin_to_core: false,
      backlog: 1024,
      drain_timeout: Duration::from_secs(30),
      header_read_timeout: Some(Duration::from_secs(30)),
      max_connections: None,
      balance_connections: true,
    }
  }
}

fn num_cpus() -> usize {
  std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
}

#[cfg(test)]
mod tests {
  use super::PerThreadConfig;

  #[test]
  fn workers_are_not_pinned_by_default() {
    assert!(!PerThreadConfig::default().pin_to_core);
  }
}
