//! Shared TLS PEM loading helpers used by every TLS-capable Tako transport.
//!
//! Previously each `serve_tls*` / `serve_h3*` implementation carried its own
//! copy of `load_certs` / `load_key`, and `tako_streams::webtransport` reached
//! across crates into `tako_server::server_h3`. This module hosts the single
//! authoritative implementation. Both functions accept PKCS#8, PKCS#1 (RSA),
//! and SEC1 (EC) PEM blocks.

use std::fs::File;
use std::io::BufReader;

use rustls::pki_types::CertificateDer;
use rustls::pki_types::PrivateKeyDer;
use rustls_pemfile::certs;
use rustls_pemfile::private_key;

/// Loads X.509 certificates from a PEM file.
pub fn load_certs(path: &str) -> anyhow::Result<Vec<CertificateDer<'static>>> {
  let mut rd = BufReader::new(
    File::open(path).map_err(|e| anyhow::anyhow!("failed to open cert file '{}': {}", path, e))?,
  );
  certs(&mut rd)
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| anyhow::anyhow!("failed to parse certs from '{}': {}", path, e))
}

/// Loads the first PEM-encoded private key from a file.
///
/// Accepts PKCS#8, PKCS#1 (RSA), and SEC1 (EC) PEM blocks.
pub fn load_key(path: &str) -> anyhow::Result<PrivateKeyDer<'static>> {
  let mut rd = BufReader::new(
    File::open(path).map_err(|e| anyhow::anyhow!("failed to open key file '{}': {}", path, e))?,
  );
  private_key(&mut rd)
    .map_err(|e| anyhow::anyhow!("bad private key in '{}': {}", path, e))?
    .ok_or_else(|| {
      anyhow::anyhow!(
        "no PEM private key (PKCS#8, PKCS#1 or SEC1) found in '{}'",
        path
      )
    })
}
