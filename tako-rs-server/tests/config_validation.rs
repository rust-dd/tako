#![cfg(not(feature = "compio"))]

use tako_rs_core::router::Router;
use tako_rs_server::Server;
use tako_rs_server::ServerConfig;

#[tokio::test]
async fn unrepresentable_timeouts_are_rejected_before_http_startup() {
  let cases = [
    (
      "h3_max_idle_timeout",
      ServerConfig {
        h3_max_idle_timeout: Some(std::time::Duration::MAX),
        ..ServerConfig::default()
      },
    ),
    (
      "drain_timeout",
      ServerConfig {
        drain_timeout: std::time::Duration::MAX,
        ..ServerConfig::default()
      },
    ),
  ];
  for (field, config) in cases {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let result = Server::builder()
      .config(config)
      .build()
      .try_spawn_http(listener, Router::new());
    assert!(result.is_err(), "invalid {field} started HTTP");
  }
}

#[tokio::test]
async fn invalid_connection_limit_is_rejected_before_http_startup() {
  for limit in [0, tokio::sync::Semaphore::MAX_PERMITS + 1] {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let result = Server::builder()
      .config(ServerConfig {
        max_connections: Some(limit),
        ..ServerConfig::default()
      })
      .build()
      .try_spawn_http(listener, Router::new());
    assert!(
      result.is_err(),
      "invalid max_connections {limit} started HTTP"
    );
  }
}

#[test]
fn invalid_configuration_is_rejected_before_raw_tcp_binding() {
  let result = Server::builder()
    .config(ServerConfig {
      max_connections: Some(0),
      ..ServerConfig::default()
    })
    .build()
    .try_spawn_tcp_raw("invalid socket address", |_, _| Box::pin(async { Ok(()) }));
  let error = result.expect_err("invalid configuration must fail");
  assert!(
    error.to_string().contains("max_connections"),
    "configuration must be checked before attempting to bind: {error}"
  );
}
