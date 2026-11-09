use std::time::Duration;

#[cfg(feature = "compio")]
use tako_rs_server::CompioServer as Server;
#[cfg(not(feature = "compio"))]
use tako_rs_server::Server;
use tako_rs_server::ServerConfig;
use tako_rs_server::ServerConfigError;

#[test]
fn fallible_builder_reports_the_invalid_field_without_a_runtime() {
  for limit in [0, tokio::sync::Semaphore::MAX_PERMITS + 1] {
    let error = Server::builder()
      .config(ServerConfig {
        max_connections: Some(limit),
        ..ServerConfig::default()
      })
      .try_build()
      .unwrap_err();
    assert_eq!(error.field(), "max_connections");
  }
  Server::builder().try_build().unwrap();
}

#[test]
fn raw_startup_returns_typed_validation_errors_before_binding() {
  let error = Server::builder()
    .config(ServerConfig {
      max_connections: Some(0),
      ..ServerConfig::default()
    })
    .build()
    .try_spawn_tcp_raw("invalid socket address", |_, _| Box::pin(async { Ok(()) }))
    .err()
    .unwrap();
  assert_eq!(
    error.downcast_ref::<ServerConfigError>().unwrap().field(),
    "max_connections"
  );
}

#[test]
fn supported_limit_boundaries_and_immediate_timeout_policies_remain_valid() {
  let config = ServerConfig {
    drain_timeout: Duration::ZERO,
    header_read_timeout: Some(Duration::ZERO),
    h2_max_concurrent_streams: 1,
    h2_max_send_buf_size: u32::MAX as usize,
    h2_max_pending_accept_reset_streams: 0,
    h2_keep_alive_interval: Some(Duration::ZERO),
    h3_max_concurrent_bidi_streams: 1,
    h3_max_concurrent_uni_streams: 3,
    h3_max_idle_timeout: Some(Duration::ZERO),
    h3_goaway_grace: Duration::ZERO,
    max_connections: Some(tokio::sync::Semaphore::MAX_PERMITS),
    proxy_read_timeout: Duration::ZERO,
    tls_handshake_timeout: Duration::ZERO,
    ..ServerConfig::default()
  };
  config.validate().unwrap();
  Server::builder().config(config).try_build().unwrap();
}

#[test]
fn explicit_protocol_throttling_policies_remain_valid() {
  for uni_streams in [0, 1, 2] {
    let config = ServerConfig {
      h2_max_concurrent_streams: 0,
      h2_max_header_list_size: 0,
      h2_max_send_buf_size: 0,
      h3_max_concurrent_bidi_streams: 0,
      h3_max_concurrent_uni_streams: uni_streams,
      ..ServerConfig::default()
    };
    config.validate().unwrap();
    Server::builder().config(config).try_build().unwrap();
  }
}

#[cfg(target_pointer_width = "64")]
#[test]
fn oversized_http2_send_buffer_is_rejected_before_hyper_can_panic() {
  let error = ServerConfig {
    h2_max_send_buf_size: u32::MAX as usize + 1,
    ..ServerConfig::default()
  }
  .validate()
  .unwrap_err();
  assert_eq!(error.field(), "h2_max_send_buf_size");
}

#[test]
fn overflowing_deadlines_are_rejected_with_the_field_name() {
  let cases = [
    (
      "header_read_timeout",
      ServerConfig {
        header_read_timeout: Some(Duration::MAX),
        ..ServerConfig::default()
      },
    ),
    (
      "h2_keep_alive_interval",
      ServerConfig {
        h2_keep_alive_interval: Some(Duration::MAX),
        ..ServerConfig::default()
      },
    ),
    (
      "h3_goaway_grace",
      ServerConfig {
        h3_goaway_grace: Duration::MAX,
        ..ServerConfig::default()
      },
    ),
    (
      "proxy_read_timeout",
      ServerConfig {
        proxy_read_timeout: Duration::MAX,
        ..ServerConfig::default()
      },
    ),
    (
      "tls_handshake_timeout",
      ServerConfig {
        tls_handshake_timeout: Duration::MAX,
        ..ServerConfig::default()
      },
    ),
  ];
  for (field, config) in cases {
    let error = config.validate().unwrap_err();
    assert_eq!(error.field(), field);
    let io_error = std::io::Error::from(error);
    assert_eq!(io_error.kind(), std::io::ErrorKind::InvalidInput);
  }
}
