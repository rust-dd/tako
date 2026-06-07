//! Production-readiness configuration shared by every Tako server transport.

use std::time::Duration;

/// Selectable QUIC congestion controller. Mirrors the controllers shipped by
/// `quinn::congestion`. Exposed here so HTTP/3 deployments can pick a profile
/// without depending on quinn directly from the application crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum H3Congestion {
  /// CUBIC — quinn's default and the most widely deployed.
  #[default]
  Cubic,
  /// `NewReno` — older, conservative.
  NewReno,
  /// BBR — Google's bandwidth-delay-product controller; useful on
  /// high-bandwidth, lossy links.
  Bbr,
}

/// Production-readiness knobs shared by every Tako server transport.
///
/// `Default` mirrors the historical hardcoded values (30 s drain, 30 s header
/// read, 100 H2 streams, …) so existing call sites keep their behavior. Pass
/// a populated `ServerConfig` to `*_with_config` entry points to override
/// individual knobs.
#[derive(Debug, Clone)]
pub struct ServerConfig {
  /// Maximum time the coordinator waits for in-flight connections to finish
  /// after a shutdown signal. After this elapses, remaining tasks are aborted.
  pub drain_timeout: Duration,
  /// Maximum time hyper waits for the request line + headers to arrive.
  /// `None` disables the timeout (the previous behavior).
  pub header_read_timeout: Option<Duration>,
  /// HTTP/1 keep-alive (default `true`).
  pub keep_alive: bool,
  /// HTTP/1 keep-alive idle timeout (Hyper default applies if `None`).
  pub keep_alive_timeout: Option<Duration>,
  /// HTTP/2 `SETTINGS_MAX_CONCURRENT_STREAMS` cap.
  pub h2_max_concurrent_streams: u32,
  /// HTTP/2 `SETTINGS_MAX_HEADER_LIST_SIZE` cap (bytes).
  pub h2_max_header_list_size: u32,
  /// HTTP/2 send-buffer cap per stream (bytes).
  pub h2_max_send_buf_size: usize,
  /// HTTP/2 pending-accept `RST_STREAM` cap (CVE-2023-44487 mitigation).
  pub h2_max_pending_accept_reset_streams: usize,
  /// HTTP/2 keep-alive ping interval. `None` disables.
  pub h2_keep_alive_interval: Option<Duration>,
  /// HTTP/3 cap on concurrent client-initiated bidirectional streams. Maps to
  /// `quinn::TransportConfig::max_concurrent_bidi_streams`.
  pub h3_max_concurrent_bidi_streams: u32,
  /// HTTP/3 cap on concurrent client-initiated unidirectional streams. Maps to
  /// `quinn::TransportConfig::max_concurrent_uni_streams`.
  pub h3_max_concurrent_uni_streams: u32,
  /// HTTP/3 idle-timeout (no QUIC packets in either direction). `None` lets
  /// quinn pick its default; `Some(d)` caps the connection lifetime.
  pub h3_max_idle_timeout: Option<Duration>,
  /// HTTP/3 congestion controller selection.
  pub h3_congestion: H3Congestion,
  /// Enable QUIC datagrams (RFC 9221) on HTTP/3 connections. Required for
  /// downstream WebTransport-style traffic.
  pub h3_enable_datagrams: bool,
  /// Issue a QUIC Retry packet for each new connection whose source address
  /// has not been validated. Mitigates UDP source-address-spoofing
  /// amplification attacks at the cost of one extra round-trip per new client.
  pub h3_use_retry: bool,
  /// Per-connection grace given to in-flight HTTP/3 streams to finish after
  /// the per-connection GOAWAY.
  ///
  /// The effective grace at runtime is `min(h3_goaway_grace, drain_timeout)`
  /// — the server clamps this so a long per-connection grace cannot push the
  /// total shutdown past the global drain budget. Configuring
  /// `h3_goaway_grace` larger than `drain_timeout` is therefore a no-op
  /// beyond the global ceiling.
  pub h3_goaway_grace: Duration,
  /// Optional ceiling on concurrent in-flight connections. Enforced via a
  /// semaphore in the accept loop; `None` disables.
  pub max_connections: Option<usize>,
  /// Read deadline applied before the PROXY protocol header is parsed.
  pub proxy_read_timeout: Duration,
  /// Maximum time the TLS acceptor waits for the client to complete its
  /// handshake. A slow / stalled handshake holds a `max_connections` permit
  /// open indefinitely otherwise — TLS slowloris. Default 10 seconds.
  pub tls_handshake_timeout: Duration,
  /// Backoff schedule for `accept()` errors (typically EMFILE/ENFILE).
  pub accept_backoff: AcceptBackoff,
}

impl Default for ServerConfig {
  fn default() -> Self {
    Self {
      drain_timeout: Duration::from_secs(30),
      header_read_timeout: Some(Duration::from_secs(30)),
      keep_alive: true,
      keep_alive_timeout: None,
      h2_max_concurrent_streams: 100,
      h2_max_header_list_size: 16 * 1024,
      h2_max_send_buf_size: 1024 * 1024,
      h2_max_pending_accept_reset_streams: 50,
      h2_keep_alive_interval: None,
      h3_max_concurrent_bidi_streams: 100,
      h3_max_concurrent_uni_streams: 8,
      h3_max_idle_timeout: Some(Duration::from_secs(30)),
      h3_congestion: H3Congestion::default(),
      h3_enable_datagrams: false,
      h3_use_retry: false,
      h3_goaway_grace: Duration::from_secs(10),
      max_connections: None,
      proxy_read_timeout: Duration::from_secs(10),
      tls_handshake_timeout: Duration::from_secs(10),
      accept_backoff: AcceptBackoff::new(),
    }
  }
}

/// Exponential backoff state for `listener.accept()` retry loops.
///
/// Accept errors (typically `EMFILE`/`ENFILE` when the process has run out of
/// file descriptors, or transient `ConnectionAborted` under load) are not fatal
/// to the listener. Servers should log, sleep, and re-poll. Use [`AcceptBackoff`]
/// to keep the sleep schedule consistent across transports without duplicating
/// the constants in every `serve_*` implementation.
#[derive(Debug, Clone, Copy)]
pub struct AcceptBackoff {
  current: Duration,
  max: Duration,
}

impl Default for AcceptBackoff {
  fn default() -> Self {
    Self::new()
  }
}

impl AcceptBackoff {
  /// Construct with the default 5 ms → 1 s schedule.
  #[must_use]
  pub const fn new() -> Self {
    Self {
      current: Duration::from_millis(5),
      max: Duration::from_secs(1),
    }
  }

  /// Reset the schedule after a successful accept.
  #[inline]
  pub fn reset(&mut self) {
    self.current = Duration::from_millis(5);
  }

  /// Sleep for the current backoff and double it (capped at `max`).
  /// Use the tokio `sleep` so this is cooperative on the runtime that runs
  /// the accept loop.
  pub async fn sleep_and_grow(&mut self) {
    let d = self.current_and_grow();
    tokio::time::sleep(d).await;
  }

  /// Returns the current backoff duration and doubles the internal counter
  /// (capped at `max`). Use this when you need to drive the sleep with a
  /// non-tokio timer (e.g. `compio::time::sleep`).
  pub fn current_and_grow(&mut self) -> Duration {
    let d = self.current;
    self.current = (self.current * 2).min(self.max);
    d
  }
}
