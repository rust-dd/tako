//! Shared PEM loading for TLS transports.

use anyhow::Context;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::PrivateKeyDer;
use rustls::pki_types::pem::PemObject;

/// Loads X.509 certificates from a PEM file.
pub fn load_certs(path: &str) -> anyhow::Result<Vec<CertificateDer<'static>>> {
  CertificateDer::pem_file_iter(path)
    .with_context(|| format!("failed to open certificates: {path}"))?
    .collect::<Result<Vec<_>, _>>()
    .with_context(|| format!("failed to parse certificates: {path}"))
}

/// Loads a PKCS#8, PKCS#1 (RSA), or SEC1 (EC) private key.
pub fn load_key(path: &str) -> anyhow::Result<PrivateKeyDer<'static>> {
  PrivateKeyDer::from_pem_file(path).with_context(|| format!("failed to load private key: {path}"))
}
