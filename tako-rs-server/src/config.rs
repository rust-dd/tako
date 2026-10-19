//! Limits and timeouts shared by every server transport.

use std::time::Duration;

/// QUIC congestion controller for HTTP/3, from `quinn::congestion` without a quinn dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum H3Congestion {
  /// CUBIC — quinn's default and the most widely deployed.
  #[default]
  Cubic,
  /// `NewReno` — older, conservative.
  NewReno,
  /// BBR — bandwidth-delay-product based; suits high-bandwidth, lossy links.
  Bbr,
}

/// Limits and timeouts shared by every transport; pass it to the server builder's `config`.
#[derive(Debug, Clone)]
pub struct ServerConfig {
  /// How long shutdown waits for in-flight connections before aborting them, 30 s by default.
  pub drain_timeout: Duration,
  /// How long a connection may go without a complete request head (30 s; `None` disables it).
  /// Plain HTTP/1 checks every half deadline, so an idle connection closes after 1-1.5x this.
  pub header_read_timeout: Option<Duration>,
  /// HTTP/1 keep-alive (default `true`).
  pub keep_alive: bool,

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
  /// HTTP/3 idle timeout without QUIC packets in either direction; `None` keeps quinn's default.
  pub h3_max_idle_timeout: Option<Duration>,
  /// HTTP/3 congestion controller, CUBIC by default.
  pub h3_congestion: H3Congestion,
  /// QUIC datagrams (RFC 9221) on HTTP/3 connections; always on with `webtransport`, whose
  /// sessions require them.
  pub h3_enable_datagrams: bool,
  /// Sends a QUIC Retry to every unvalidated client address, which stops spoofed-source
  /// amplification at the cost of one round-trip per new client.
  pub h3_use_retry: bool,
  /// How long in-flight HTTP/3 streams may finish after a connection's GOAWAY; capped at
  /// `drain_timeout`.
  pub h3_goaway_grace: Duration,
  /// Cap on concurrent connections, `None` for none; an upgraded connection such as a WebSocket
  /// counts until its socket closes.
  pub max_connections: Option<usize>,
  /// Read deadline applied before the PROXY protocol header is parsed, 10 s by default.
  pub proxy_read_timeout: Duration,
  /// Deadline for the client's TLS handshake, 10 s by default, so a stalled one (TLS slowloris)
  /// cannot hold a `max_connections` slot.
  pub tls_handshake_timeout: Duration,
  /// Backoff schedule for `accept()` errors such as EMFILE.
  pub accept_backoff: AcceptBackoff,
}

impl Default for ServerConfig {
  fn default() -> Self {
    Self {
      drain_timeout: Duration::from_secs(30),
      header_read_timeout: Some(Duration::from_secs(30)),
      keep_alive: true,
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

/// Exponential backoff for accept loops, whose errors (e.g. `EMFILE`) are transient rather than
/// fatal to the listener.
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
  /// Starts at 5 ms and doubles up to 1 s.
  #[must_use]
  pub const fn new() -> Self {
    Self {
      current: Duration::from_millis(5),
      max: Duration::from_secs(1),
    }
  }

  /// Resets the schedule after a successful accept.
  #[inline]
  pub fn reset(&mut self) {
    self.current = Duration::from_millis(5);
  }

  /// Sleeps for the current backoff on Tokio's timer and doubles it up to the cap.
  pub async fn sleep_and_grow(&mut self) {
    let d = self.current_and_grow();
    tokio::time::sleep(d).await;
  }

  /// Returns the current backoff and doubles it up to the cap, for loops that sleep on another
  /// timer such as `compio::time::sleep`.
  pub fn current_and_grow(&mut self) -> Duration {
    let d = self.current;
    self.current = (self.current * 2).min(self.max);
    d
  }
}
