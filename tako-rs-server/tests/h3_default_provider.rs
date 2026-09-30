#![cfg(feature = "http3")]

//! Runs in its own test binary because the rustls crypto provider is
//! process-wide state: no other test may install one first.

use std::time::Duration;

use tako_rs_core::router::Router;
use tako_rs_server::TlsCert;

fn tls() -> TlsCert {
  let identity = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
  let key = rustls::pki_types::PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der());
  TlsCert::der(vec![identity.cert.der().clone()], key.into())
}

#[cfg(not(feature = "compio"))]
#[tokio::test]
async fn h3_starts_without_a_preinstalled_crypto_provider() {
  let handle = tako_rs_server::Server::builder()
    .tls(tls())
    .build()
    .try_spawn_h3("127.0.0.1:0", Router::new())
    .unwrap();
  handle.shutdown(Duration::from_millis(100)).await;
}

#[cfg(feature = "compio")]
#[test]
fn h3_starts_without_a_preinstalled_crypto_provider() {
  compio::runtime::Runtime::new().unwrap().block_on(async {
    let handle = tako_rs_server::CompioServer::builder()
      .tls(tls())
      .build()
      .try_spawn_h3("127.0.0.1:0", Router::new())
      .unwrap();
    handle.shutdown(Duration::from_millis(100)).await;
  });
}
