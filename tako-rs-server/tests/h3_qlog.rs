#![cfg(feature = "http3-qlog")]

use std::collections::HashSet;
use std::future::Future;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use rustls::pki_types::CertificateDer;
use tako_rs_core::router::Router;
#[cfg(feature = "compio")]
use tako_rs_server::CompioServer as Server;
#[cfg(not(feature = "compio"))]
use tako_rs_server::Server;
use tako_rs_server::TlsCert;

#[path = "support/h3.rs"]
#[allow(dead_code)]
mod h3;

#[test]
fn each_connection_writes_a_distinct_usable_trace() {
  let directory = TestDirectory::new();
  run(Some(&directory.0), |address, certificate| async move {
    let (first, second) = tokio::join!(
      h3::Client::new(address, certificate.clone()),
      h3::Client::new(address, certificate)
    );
    tokio::join!(request(first), request(second));
  });
  assert_traces(&directory.0, 2);
}

#[test]
fn logging_is_disabled_without_a_directory() {
  let directory = TestDirectory::new();
  let output = std::process::Command::new(std::env::current_exe().unwrap())
    .args(["--ignored", "--exact", "default_off_process"])
    .env("QLOGDIR", &directory.0)
    .current_dir(&directory.0)
    .output()
    .unwrap();
  assert!(
    output.status.success(),
    "{}",
    String::from_utf8_lossy(&output.stderr)
  );
  assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
}

#[test]
#[ignore = "runs in an isolated process with QLOGDIR set"]
fn default_off_process() {
  run(None, |address, certificate| async move {
    request(h3::Client::new(address, certificate).await).await;
  });
}

#[cfg(feature = "webtransport")]
#[test]
fn webtransport_sessions_keep_the_connection_trace() {
  let directory = TestDirectory::new();
  run(Some(&directory.0), |address, certificate| async move {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(certificate).unwrap();
    let mut tls = rustls::ClientConfig::builder()
      .with_root_certificates(roots)
      .with_no_client_auth();
    tls.alpn_protocols = vec![b"h3".to_vec()];
    let config = wtransport::ClientConfig::builder()
      .with_bind_default()
      .with_custom_tls(tls)
      .build();
    let endpoint = wtransport::Endpoint::client(config).unwrap();
    let connection = endpoint
      .connect(format!("https://{address}/wt"))
      .await
      .unwrap();
    let mut stream = connection.accept_uni().await.unwrap();
    let mut text = String::new();
    tokio::io::AsyncReadExt::read_to_string(&mut stream, &mut text)
      .await
      .unwrap();
    assert_eq!(text, "traced session");
    connection.close(0u32.into(), b"finished");
    endpoint.wait_idle().await;
  });
  assert_traces(&directory.0, 1);
}

#[test]
fn invalid_destinations_fail_before_starting_the_server() {
  let directory = TestDirectory::new();
  let file = directory.0.join("file");
  std::fs::write(&file, b"keep").unwrap();
  for path in [file.clone(), directory.0.join("missing"), "".into()] {
    let (_, tls) = identity();
    let result = Server::builder()
      .tls(tls)
      .h3_qlog_dir(path)
      .build()
      .try_spawn_h3("127.0.0.1:0", Router::new());
    assert!(result.is_err(), "invalid qlog destination was accepted");
  }
  assert_eq!(std::fs::read(file).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn a_read_only_directory_is_rejected_at_startup() {
  use std::os::unix::fs::PermissionsExt;

  let directory = TestDirectory::new();
  std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o500)).unwrap();
  // Privileged test runners can bypass directory permissions.
  let probe = directory.0.join("permission-probe");
  if std::fs::write(&probe, []).is_ok() {
    std::fs::remove_file(probe).unwrap();
    std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o700)).unwrap();
    return;
  }
  let (_, tls) = identity();
  let result = Server::builder()
    .tls(tls)
    .h3_qlog_dir(&directory.0)
    .build()
    .try_spawn_h3("127.0.0.1:0", Router::new());
  std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o700)).unwrap();
  assert!(result.is_err(), "read-only qlog directory was accepted");
}

#[test]
fn a_destination_failure_does_not_interrupt_http3() {
  let directory = TestDirectory::new();
  let path = directory.0.clone();
  run(Some(&directory.0), |address, certificate| async move {
    std::fs::remove_dir(path).unwrap();
    request(h3::Client::new(address, certificate).await).await;
  });
  assert!(!directory.0.exists());
}

async fn request(mut client: h3::Client) {
  let mut stream = client
    .sender
    .send_request(
      http::Request::builder()
        .uri("https://localhost/")
        .body(())
        .unwrap(),
    )
    .await
    .unwrap();
  stream.finish().await.unwrap();
  assert_eq!(stream.recv_response().await.unwrap().status(), 200);
  while stream.recv_data().await.unwrap().is_some() {}
  client.close().await;
}

fn identity() -> (CertificateDer<'static>, TlsCert) {
  let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
  let identity =
    rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
  let certificate = identity.cert.der().clone();
  let key = rustls::pki_types::PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der());
  let tls = TlsCert::der(vec![certificate.clone()], key.into());
  (certificate, tls)
}

fn run<F, Fut>(directory: Option<&Path>, test: F)
where
  F: FnOnce(SocketAddr, CertificateDer<'static>) -> Fut + Send + 'static,
  Fut: Future<Output = ()> + Send + 'static,
{
  let (certificate, tls) = identity();
  let mut builder = Server::builder()
    .tls(tls)
    .config(tako_rs_server::ServerConfig {
      h3_use_retry: true,
      ..tako_rs_server::ServerConfig::default()
    });
  if let Some(directory) = directory {
    builder = builder.h3_qlog_dir(directory);
  }
  let mut router = Router::new();
  router.get("/", || async { "qlog" });
  #[cfg(feature = "webtransport")]
  router.route(
    http::Method::CONNECT,
    "/wt",
    |wt: tako_rs_server::webtransport::WebTransport| async {
      wt.on_session(|session| async move {
        let mut stream = session.open_uni().await.unwrap();
        tokio::io::AsyncWriteExt::write_all(&mut stream, b"traced session")
          .await
          .unwrap();
        tokio::io::AsyncWriteExt::shutdown(&mut stream)
          .await
          .unwrap();
        session.closed().await;
      })
    },
  );
  #[cfg(not(feature = "compio"))]
  tokio::runtime::Runtime::new().unwrap().block_on(async {
    let server = builder.build().try_spawn_h3("127.0.0.1:0", router).unwrap();
    assert_no_startup_files(directory);
    tokio::time::timeout(
      Duration::from_secs(10),
      test(server.local_addr().unwrap(), certificate),
    )
    .await
    .unwrap();
    server.trigger();
    server.result().await.unwrap();
  });
  #[cfg(feature = "compio")]
  compio::runtime::Runtime::new().unwrap().block_on(async {
    let server = builder.build().try_spawn_h3("127.0.0.1:0", router).unwrap();
    assert_no_startup_files(directory);
    let address = server.local_addr().unwrap();
    let (done, received) = tokio::sync::oneshot::channel();
    let thread = std::thread::spawn(move || {
      tokio::runtime::Runtime::new().unwrap().block_on(async {
        tokio::time::timeout(Duration::from_secs(10), test(address, certificate))
          .await
          .unwrap();
      });
      let _ = done.send(());
    });
    let result = compio::time::timeout(Duration::from_secs(15), received).await;
    server.trigger();
    server.result().await.unwrap();
    thread.join().unwrap();
    result.unwrap().unwrap();
  });
}

fn assert_no_startup_files(directory: Option<&Path>) {
  if let Some(directory) = directory {
    assert_eq!(std::fs::read_dir(directory).unwrap().count(), 0);
  }
}

fn assert_traces(directory: &Path, count: usize) {
  let files = std::fs::read_dir(directory)
    .unwrap()
    .map(|entry| entry.unwrap().path())
    .collect::<Vec<_>>();
  assert_eq!(files.len(), count);
  let mut connections = HashSet::new();
  for file in files {
    assert_eq!(file.extension().unwrap(), "sqlog");
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o077,
        0
      );
    }
    let contents = std::fs::read(&file).unwrap();
    assert_eq!(contents[0], 0x1e);
    let records = contents
      .split(|byte| *byte == 0x1e)
      .skip(1)
      .map(|record| serde_json::from_slice::<serde_json::Value>(record).unwrap())
      .collect::<Vec<_>>();
    assert_eq!(records[0]["qlog_format"], "JSON-SEQ");
    assert!(
      records
        .iter()
        .any(|record| record["name"] == "transport:packet_sent")
    );
    assert!(
      records
        .iter()
        .any(|record| record["name"] == "transport:packet_received")
    );
    let groups = records
      .iter()
      .filter_map(|record| record["group_id"].as_str())
      .collect::<HashSet<_>>();
    assert_eq!(
      groups.len(),
      1,
      "connections shared a trace: {}",
      file.display()
    );
    assert!(connections.insert((*groups.iter().next().unwrap()).to_owned()));
  }
}

struct TestDirectory(std::path::PathBuf);

impl TestDirectory {
  fn new() -> Self {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("tako-qlog-test-{}-{id}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    Self(path)
  }
}

impl Drop for TestDirectory {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.0);
  }
}
