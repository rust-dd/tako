use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use rustls::pki_types::CertificateDer;
use tako_rs_core::router::Router;
use tako_rs_server::TlsCert;

pub struct Client {
  pub sender: h3::client::SendRequest<h3_quinn::OpenStreams, Bytes>,
  endpoint: quinn::Endpoint,
  driver: tokio::task::JoinHandle<h3::error::ConnectionError>,
}

impl Client {
  pub async fn new(address: SocketAddr, certificate: CertificateDer<'static>) -> Self {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(certificate).unwrap();
    let mut tls = rustls::ClientConfig::builder()
      .with_root_certificates(roots)
      .with_no_client_auth();
    tls.alpn_protocols = vec![b"h3".to_vec()];
    let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
    endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(
      quinn::crypto::rustls::QuicClientConfig::try_from(tls).unwrap(),
    )));
    let connection = endpoint
      .connect(address, "localhost")
      .unwrap()
      .await
      .unwrap();
    let (mut driver, sender) = h3::client::new(h3_quinn::Connection::new(connection))
      .await
      .unwrap();
    let driver =
      tokio::spawn(async move { std::future::poll_fn(|cx| driver.poll_close(cx)).await });
    Self {
      sender,
      endpoint,
      driver,
    }
  }

  pub async fn close(self) {
    self.endpoint.close(0u32.into(), b"finished");
    self.driver.await.unwrap();
  }
}

pub fn run<F, Fut>(router: Router, test: F)
where
  F: FnOnce(Client) -> Fut + Send + 'static,
  Fut: Future<Output = ()> + Send + 'static,
{
  let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
  let identity = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
  let certificate = identity.cert.der().clone();
  let key = rustls::pki_types::PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der());
  let cert = TlsCert::der(vec![certificate.clone()], key.into());
  #[cfg(not(feature = "compio"))]
  tokio::runtime::Runtime::new().unwrap().block_on(async {
    let server = tako_rs_server::Server::builder()
      .tls(cert)
      .build()
      .try_spawn_h3("127.0.0.1:0", router)
      .unwrap();
    let client = Client::new(server.local_addr().unwrap(), certificate).await;
    tokio::time::timeout(Duration::from_secs(5), test(client))
      .await
      .unwrap();
    server.shutdown(Duration::from_secs(1)).await;
  });
  #[cfg(feature = "compio")]
  compio::runtime::Runtime::new().unwrap().block_on(async {
    let server = tako_rs_server::CompioServer::builder()
      .tls(cert)
      .build()
      .try_spawn_h3("127.0.0.1:0", router)
      .unwrap();
    let address = server.local_addr().unwrap();
    let (done, received) = tokio::sync::oneshot::channel();
    let thread = std::thread::spawn(move || {
      tokio::runtime::Runtime::new().unwrap().block_on(async {
        let client = Client::new(address, certificate).await;
        tokio::time::timeout(Duration::from_secs(5), test(client))
          .await
          .unwrap();
      });
      let _ = done.send(());
    });
    let result = compio::time::timeout(Duration::from_secs(8), received).await;
    server.shutdown(Duration::from_secs(1)).await;
    thread.join().unwrap();
    result.unwrap().unwrap();
  });
}
