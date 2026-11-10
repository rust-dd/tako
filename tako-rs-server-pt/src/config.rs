use std::time::Duration;
use std::time::Instant;

/// Configuration for [`serve_per_thread`](crate::serve_per_thread) and
/// [`spawn_per_thread`](crate::spawn_per_thread).
#[derive(Debug, Clone)]
pub struct PerThreadConfig {
  /// Number of worker threads. Defaults to the number of logical CPUs.
  pub workers: usize,
  /// Pins worker N to core N (`affinity` feature). Off by default, since a pinned worker cannot
  /// leave a core another pinning process keeps busy; use it when the server owns the machine.
  pub pin_to_core: bool,
  /// Positive `SO_REUSEPORT` listen backlog.
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

impl PerThreadConfig {
  /// Rejects invalid counts and deadlines. Zero timeouts expire immediately and are valid.
  pub fn validate(&self) -> std::io::Result<()> {
    let invalid = |message| std::io::Error::new(std::io::ErrorKind::InvalidInput, message);

    if self.workers == 0 {
      return Err(invalid("workers must be positive"));
    }

    if self.backlog <= 0 {
      return Err(invalid("backlog must be positive"));
    }

    if let Some(limit) = self.max_connections
      && (limit == 0 || limit > tokio::sync::Semaphore::MAX_PERMITS)
    {
      return Err(invalid(
        "max_connections must be between 1 and Semaphore::MAX_PERMITS",
      ));
    }

    let now = Instant::now();
    if now.checked_add(self.drain_timeout).is_none()
      || self
        .header_read_timeout
        .is_some_and(|timeout| now.checked_add(timeout).is_none())
    {
      return Err(invalid(
        "timeouts must fit within the monotonic clock's range",
      ));
    }

    Ok(())
  }
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
