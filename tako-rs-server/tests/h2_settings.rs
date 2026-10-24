#![cfg(all(
  feature = "http2",
  any(all(feature = "tls", not(feature = "compio")), feature = "compio-tls")
))]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::CertificateDer;
use tako_rs_core::router::Router;
#[cfg(feature = "compio")]
use tako_rs_server::CompioServer as RuntimeServer;
#[cfg(not(feature = "compio"))]
use tako_rs_server::Server as RuntimeServer;
use tako_rs_server::ServerConfig;
use tako_rs_server::TlsCert;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;

#[test]
fn h2c_applies_configured_settings_and_preserves_flow_control_defaults() {
  run(false);
}

#[test]
fn tls_applies_configured_settings_and_preserves_flow_control_defaults() {
  run(true);
}

fn run(tls: bool) {
  let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
  #[cfg(not(feature = "compio"))]
  tokio::runtime::Runtime::new()
    .unwrap()
    .block_on(exercise(tls));
  #[cfg(feature = "compio")]
  compio::runtime::Runtime::new()
    .unwrap()
    .block_on(exercise(tls));
}

async fn exercise(tls: bool) {
  let identity = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
  let certificate = identity.cert.der().clone();
  let key = rustls::pki_types::PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der());
  let config = ServerConfig {
    h2_max_concurrent_streams: 7,
    h2_max_header_list_size: 12_345,
    h2_max_send_buf_size: 16_384,
    h2_max_pending_accept_reset_streams: 17,
    h2_keep_alive_interval: Some(Duration::from_millis(20)),
    drain_timeout: Duration::from_millis(100),
    ..ServerConfig::default()
  };
  #[cfg(not(feature = "compio"))]
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  #[cfg(feature = "compio")]
  let listener = compio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let server = RuntimeServer::builder()
    .config(config)
    .tls(TlsCert::der(vec![certificate.clone()], key.into()))
    .build();
  let handle = if tls {
    server.spawn_tls(listener, Router::new())
  } else {
    server.spawn_h2c(listener, Router::new())
  };
  let (done, finished) = tokio::sync::oneshot::channel();
  let client = std::thread::spawn(move || {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
      tokio::time::timeout(Duration::from_secs(2), exchange(address, certificate, tls))
        .await
        .unwrap();
    });
    let _ = done.send(());
  });
  #[cfg(not(feature = "compio"))]
  let result = tokio::time::timeout(Duration::from_secs(3), finished).await;
  #[cfg(feature = "compio")]
  let result = compio::time::timeout(Duration::from_secs(3), finished).await;
  handle.shutdown(Duration::from_secs(1)).await;
  client.join().unwrap();
  result.unwrap().unwrap();
}

async fn exchange(address: SocketAddr, certificate: CertificateDer<'static>, tls: bool) {
  let stream = tokio::net::TcpStream::connect(address).await.unwrap();
  if tls {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(certificate).unwrap();
    let mut config = rustls::ClientConfig::builder()
      .with_root_certificates(roots)
      .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec()];
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let stream = connector
      .connect(
        rustls::pki_types::ServerName::try_from("localhost").unwrap(),
        stream,
      )
      .await
      .unwrap();
    assert_eq!(stream.get_ref().1.alpn_protocol(), Some(b"h2".as_slice()));
    inspect_wire(stream).await;
  } else {
    inspect_wire(stream).await;
  }
}

async fn inspect_wire(mut stream: impl AsyncRead + AsyncWrite + Unpin) {
  stream
    .write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n\x00\x00\x00\x04\x00\x00\x00\x00\x00")
    .await
    .unwrap();
  let mut settings = std::collections::BTreeMap::new();
  let mut connection_window = 65_535;
  loop {
    let mut head = [0; 9];
    stream.read_exact(&mut head).await.unwrap();
    let length = usize::from(head[0]) << 16 | usize::from(head[1]) << 8 | usize::from(head[2]);
    let mut payload = vec![0; length];
    stream.read_exact(&mut payload).await.unwrap();
    match head[3] {
      4 if head[4] == 0 => {
        for setting in payload.as_chunks::<6>().0 {
          settings.insert(
            u16::from_be_bytes([setting[0], setting[1]]),
            u32::from_be_bytes(setting[2..6].try_into().unwrap()),
          );
        }
        stream
          .write_all(&[0, 0, 0, 4, 1, 0, 0, 0, 0])
          .await
          .unwrap();
      }
      8 if head[5..9] == [0, 0, 0, 0] => {
        connection_window += u32::from_be_bytes(payload.try_into().unwrap());
      }
      6 if head[4] == 0 => {
        assert_eq!(payload.len(), 8);
        break;
      }
      _ => {}
    }
  }
  assert_eq!(settings.get(&3), Some(&7));
  assert_eq!(settings.get(&6), Some(&12_345));
  assert_eq!(settings.get(&4), Some(&1_048_576));
  assert_eq!(settings.get(&5), Some(&16_384));
  assert_eq!(connection_window, 1_048_576);
}
