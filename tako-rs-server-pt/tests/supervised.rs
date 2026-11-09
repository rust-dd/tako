use std::time::Duration;

use tako_rs_core::router::Router;
use tako_rs_server_pt::PerThreadConfig;
use tako_rs_server_pt::PerThreadErrorKind;
use tako_rs_server_pt::PerThreadWorkerState;
use tako_rs_server_pt::spawn_per_thread_supervised;

#[tokio::test]
async fn clean_shutdown_clears_readiness_and_joins_every_worker() {
  let handle = spawn_per_thread_supervised(
    "127.0.0.1:0",
    Router::new(),
    PerThreadConfig {
      workers: 2,
      ..PerThreadConfig::default()
    },
  )
  .unwrap();
  tokio::time::timeout(Duration::from_secs(2), handle.wait_ready())
    .await
    .unwrap()
    .unwrap();
  let readiness = handle.readiness();
  assert!(readiness.is_ready());
  handle.shutdown();
  assert!(!readiness.is_ready());
  tokio::time::timeout(Duration::from_secs(2), handle.result())
    .await
    .unwrap()
    .unwrap();
  handle.join().unwrap();
  assert_eq!(readiness.workers(), [PerThreadWorkerState::Stopped; 2]);
}

#[tokio::test]
async fn every_bind_failing_reports_an_error_instead_of_waiting_forever() {
  let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
  for balance_connections in [false, true] {
    let handle = spawn_per_thread_supervised(
      &occupied.local_addr().unwrap().to_string(),
      Router::new(),
      PerThreadConfig {
        workers: 2,
        balance_connections,
        ..PerThreadConfig::default()
      },
    )
    .unwrap();
    let ready = tokio::time::timeout(Duration::from_secs(2), handle.wait_ready()).await;
    let result = tokio::time::timeout(Duration::from_secs(2), handle.result()).await;
    handle.shutdown();
    let joined = handle.join();
    assert!(ready.unwrap().is_err());
    assert_eq!(
      result.unwrap().unwrap_err().kind(),
      PerThreadErrorKind::Bind
    );
    assert_eq!(joined.unwrap_err().kind(), PerThreadErrorKind::Bind);
  }
}

#[test]
fn invalid_configuration_fails_before_workers_are_spawned() {
  let invalid = [
    PerThreadConfig {
      max_connections: Some(tokio::sync::Semaphore::MAX_PERMITS + 1),
      ..PerThreadConfig::default()
    },
    PerThreadConfig {
      backlog: -1,
      ..PerThreadConfig::default()
    },
    PerThreadConfig {
      drain_timeout: Duration::MAX,
      ..PerThreadConfig::default()
    },
    PerThreadConfig {
      header_read_timeout: Some(Duration::MAX),
      ..PerThreadConfig::default()
    },
  ];
  for config in invalid {
    let spawned = spawn_per_thread_supervised("127.0.0.1:0", Router::new(), config);
    assert!(matches!(spawned, Err(error) if error.kind() == std::io::ErrorKind::InvalidInput));
  }
}
