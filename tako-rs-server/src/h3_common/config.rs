use std::sync::Arc;

use quinn::VarInt;
use quinn::congestion::BbrConfig;
use quinn::congestion::CubicConfig;
use quinn::congestion::NewRenoConfig;

use crate::H3Congestion;
use crate::ServerConfig;

/// Build a `quinn::TransportConfig` from the H3-specific knobs in [`ServerConfig`].
pub(crate) fn transport_config_from(config: &ServerConfig) -> quinn::TransportConfig {
  let mut tc = quinn::TransportConfig::default();
  tc.max_concurrent_bidi_streams(VarInt::from_u32(config.h3_max_concurrent_bidi_streams));
  tc.max_concurrent_uni_streams(VarInt::from_u32(config.h3_max_concurrent_uni_streams));
  if let Some(idle) = config.h3_max_idle_timeout
    && let Ok(idle) = idle.try_into()
  {
    tc.max_idle_timeout(Some(idle));
  }
  // QUIC datagrams (RFC 9221). Required for downstream WebTransport-style
  // traffic. Send buffer is left at the quinn default.
  if config.h3_enable_datagrams {
    tc.datagram_receive_buffer_size(Some(64 * 1024));
  } else {
    tc.datagram_receive_buffer_size(None);
  }
  match config.h3_congestion {
    H3Congestion::Cubic => {
      tc.congestion_controller_factory(Arc::new(CubicConfig::default()));
    }
    H3Congestion::NewReno => {
      tc.congestion_controller_factory(Arc::new(NewRenoConfig::default()));
    }
    H3Congestion::Bbr => {
      tc.congestion_controller_factory(Arc::new(BbrConfig::default()));
    }
  }
  tc
}
