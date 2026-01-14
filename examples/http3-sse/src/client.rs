use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use bytes::Buf;
use quinn::crypto::rustls::QuicClientConfig;

#[derive(Debug)]
struct SkipServerVerification;

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
  fn verify_server_cert(
    &self,
    _end_entity: &rustls::pki_types::CertificateDer<'_>,
    _intermediates: &[rustls::pki_types::CertificateDer<'_>],
    _server_name: &rustls::pki_types::ServerName<'_>,
    _ocsp_response: &[u8],
    _now: rustls::pki_types::UnixTime,
  ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
    Ok(rustls::client::danger::ServerCertVerified::assertion())
  }

  fn verify_tls12_signature(
    &self,
    _message: &[u8],
    _cert: &rustls::pki_types::CertificateDer<'_>,
    _dss: &rustls::DigitallySignedStruct,
  ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
    Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
  }

  fn verify_tls13_signature(
    &self,
    _message: &[u8],
    _cert: &rustls::pki_types::CertificateDer<'_>,
    _dss: &rustls::DigitallySignedStruct,
  ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
    Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
  }

  fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
    vec![
      rustls::SignatureScheme::RSA_PKCS1_SHA256,
      rustls::SignatureScheme::RSA_PKCS1_SHA384,
      rustls::SignatureScheme::RSA_PKCS1_SHA512,
      rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
      rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
      rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
      rustls::SignatureScheme::RSA_PSS_SHA256,
      rustls::SignatureScheme::RSA_PSS_SHA384,
      rustls::SignatureScheme::RSA_PSS_SHA512,
      rustls::SignatureScheme::ED25519,
    ]
  }
}

#[tokio::main]
async fn main() -> Result<()> {
  let _ = rustls::crypto::ring::default_provider().install_default();

  let addr = std::env::args()
    .nth(1)
    .unwrap_or_else(|| "[::1]:4433".to_string());
  let path = std::env::args()
    .nth(2)
    .unwrap_or_else(|| "/count".to_string());

  println!("Connecting to {} path {}", addr, path);

  let mut client_crypto = Arc::new(
    rustls::ClientConfig::builder()
      .dangerous()
      .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
      .with_no_client_auth(),
  );
  Arc::get_mut(&mut client_crypto).unwrap().alpn_protocols = vec![b"h3".to_vec()];

  let client_config =
    quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(client_crypto)?));

  let mut endpoint = quinn::Endpoint::client("[::]:0".parse()?)?;
  endpoint.set_default_client_config(client_config);

  let conn = endpoint.connect(addr.parse()?, "localhost")?.await?;
  println!("Connected!");

  let (mut driver, mut send_request) =
    h3::client::new(h3_quinn::Connection::new(conn)).await?;

  tokio::spawn(async move {
    let _ = std::future::poll_fn(|cx| driver.poll_close(cx)).await;
  });

  let req = http::Request::builder()
    .method("GET")
    .uri(format!("https://localhost{}", path))
    .body(())?;

  let mut stream = send_request.send_request(req).await?;
  stream.finish().await?;

  let resp = stream.recv_response().await?;
  println!("Response: {} {:?}", resp.status(), resp.headers());

  println!("\n--- SSE Data ---");
  let timeout = tokio::time::timeout(Duration::from_secs(10), async {
    loop {
      match stream.recv_data().await {
        Ok(Some(mut chunk)) => {
          while chunk.has_remaining() {
            let bytes = chunk.chunk();
            print!("{}", String::from_utf8_lossy(bytes));
            chunk.advance(bytes.len());
          }
        }
        Ok(None) => {
          println!("\n--- Stream ended ---");
          break;
        }
        Err(e) => {
          println!("\n--- Connection closed: {} ---", e);
          break;
        }
      }
    }
  });

  let _ = timeout.await;
  Ok(())
}
