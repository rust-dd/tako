use std::sync::Arc;

use quinn::VarInt;
use quinn::congestion::BbrConfig;
use quinn::congestion::CubicConfig;
use quinn::congestion::NewRenoConfig;

use crate::H3Congestion;
use crate::ServerConfig;

/// Builds the QUIC transport limits from the `h3_*` fields of [`ServerConfig`].
pub(crate) fn transport_config_from(config: &ServerConfig) -> quinn::TransportConfig {
  let mut tc = quinn::TransportConfig::default();
  tc.max_concurrent_bidi_streams(VarInt::from_u32(config.h3_max_concurrent_bidi_streams));
  tc.max_concurrent_uni_streams(VarInt::from_u32(config.h3_max_concurrent_uni_streams));
  if let Some(idle) = config.h3_max_idle_timeout
    && let Ok(idle) = idle.try_into()
  {
    tc.max_idle_timeout(Some(idle));
  }
  // WebTransport advertises HTTP/3 datagrams, which RFC 9297 allows only on top of QUIC
  // datagrams (RFC 9221).
  if config.h3_enable_datagrams || cfg!(feature = "webtransport") {
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
