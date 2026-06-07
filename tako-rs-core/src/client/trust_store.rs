//! Trust-store population for the TLS client.

use rustls::RootCertStore;
#[cfg(not(feature = "native-certs"))]
use webpki_roots::TLS_SERVER_ROOTS;

/// Populates a [`RootCertStore`] with the configured trust source.
///
/// Without the `native-certs` feature the bundled `webpki-roots` snapshot is
/// used (the historical default). With `native-certs` the operating-system
/// trust store is loaded via `rustls-native-certs`; failures during native
/// loading are logged at `warn` and silently fall through, so a missing OS
/// store does not break the client.
pub(crate) fn load_root_certs(store: &mut RootCertStore) {
  #[cfg(feature = "native-certs")]
  {
    let result = rustls_native_certs::load_native_certs();
    for err in &result.errors {
      tracing::warn!(error = %err, "rustls-native-certs partial failure");
    }
    for cert in result.certs {
      let _ = store.add(cert);
    }
  }
  #[cfg(not(feature = "native-certs"))]
  {
    store.extend(TLS_SERVER_ROOTS.iter().cloned());
  }
}
