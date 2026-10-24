use std::time::Duration;

use hyper::server::conn::http2;

use crate::ServerConfig;

/// HTTP/2 limits copied once per server and shared by the TLS and cleartext transports.
#[derive(Clone, Copy)]
pub(crate) struct H2Settings {
  max_concurrent_streams: u32,
  max_header_list_size: u32,
  max_send_buf_size: usize,
  max_pending_accept_reset_streams: usize,
  keep_alive_interval: Option<Duration>,
}

impl From<&ServerConfig> for H2Settings {
  fn from(config: &ServerConfig) -> Self {
    Self {
      max_concurrent_streams: config.h2_max_concurrent_streams,
      max_header_list_size: config.h2_max_header_list_size,
      max_send_buf_size: config.h2_max_send_buf_size,
      max_pending_accept_reset_streams: config.h2_max_pending_accept_reset_streams,
      keep_alive_interval: config.h2_keep_alive_interval,
    }
  }
}

impl H2Settings {
  pub(crate) fn apply<E>(self, builder: &mut http2::Builder<E>) {
    builder
      .max_concurrent_streams(self.max_concurrent_streams)
      .max_header_list_size(self.max_header_list_size)
      .max_send_buf_size(self.max_send_buf_size)
      .max_pending_accept_reset_streams(self.max_pending_accept_reset_streams);
    if let Some(interval) = self.keep_alive_interval {
      builder.keep_alive_interval(Some(interval));
    }
  }
}
