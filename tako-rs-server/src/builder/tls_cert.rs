#[cfg(feature = "tls")]
use std::sync::Arc;

/// mTLS policy: client certificates are verified against the [`rustls::RootCertStore`].
#[cfg(feature = "tls")]
#[derive(Clone)]
pub enum ClientAuth {
  /// Verify the client cert if presented; allow connections without one.
  Optional(Arc<rustls::RootCertStore>),
  /// Require a valid client cert; reject the handshake otherwise.
  Required(Arc<rustls::RootCertStore>),
}

#[cfg(feature = "tls")]
impl std::fmt::Debug for ClientAuth {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      ClientAuth::Optional(_) => f.debug_tuple("Optional").field(&"<root_store>").finish(),
      ClientAuth::Required(_) => f.debug_tuple("Required").field(&"<root_store>").finish(),
    }
  }
}

/// Certificate source for `spawn_tls` and `spawn_h3`; `Der` and `Resolver` need the `tls` feature.
#[derive(Clone)]
pub enum TlsCert {
  /// Filesystem paths for cert + key PEM files.
  PemPaths {
    /// Path to the PEM-encoded certificate chain.
    cert_path: String,
    /// Path to the PEM-encoded private key.
    key_path: String,
    /// Optional mTLS policy.
    #[cfg(feature = "tls")]
    client_auth: Option<ClientAuth>,
  },
  /// In-memory DER chain and key, e.g. from a secret store.
  #[cfg(feature = "tls")]
  Der {
    /// DER-encoded certificate chain (leaf first).
    certs: Arc<Vec<rustls::pki_types::CertificateDer<'static>>>,
    /// DER-encoded private key.
    key: Arc<rustls::pki_types::PrivateKeyDer<'static>>,
    /// Optional mTLS policy.
    client_auth: Option<ClientAuth>,
  },
  /// Picks a certificate per handshake: SNI multi-cert serving or hot reload through
  /// [`ReloadableResolver`].
  #[cfg(feature = "tls")]
  Resolver {
    /// The resolver used by rustls to pick a cert per handshake.
    resolver: Arc<dyn rustls::server::ResolvesServerCert>,
    /// Optional mTLS policy.
    client_auth: Option<ClientAuth>,
  },
}

impl std::fmt::Debug for TlsCert {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      TlsCert::PemPaths {
        cert_path,
        key_path,
        ..
      } => f
        .debug_struct("PemPaths")
        .field("cert_path", cert_path)
        .field("key_path", key_path)
        .finish_non_exhaustive(),
      #[cfg(feature = "tls")]
      TlsCert::Der { client_auth, .. } => f
        .debug_struct("Der")
        .field("client_auth", client_auth)
        .finish_non_exhaustive(),
      #[cfg(feature = "tls")]
      TlsCert::Resolver { client_auth, .. } => f
        .debug_struct("Resolver")
        .field("client_auth", client_auth)
        .finish_non_exhaustive(),
    }
  }
}

impl TlsCert {
  /// PEM certificate chain and key files, read when the server starts.
  pub fn pem_paths(cert: impl Into<String>, key: impl Into<String>) -> Self {
    Self::PemPaths {
      cert_path: cert.into(),
      key_path: key.into(),
      #[cfg(feature = "tls")]
      client_auth: None,
    }
  }

  /// Like [`TlsCert::pem_paths`] with an attached mTLS policy.
  #[cfg(feature = "tls")]
  pub fn pem_paths_with_client_auth(
    cert: impl Into<String>,
    key: impl Into<String>,
    client_auth: ClientAuth,
  ) -> Self {
    Self::PemPaths {
      cert_path: cert.into(),
      key_path: key.into(),
      client_auth: Some(client_auth),
    }
  }

  /// In-memory DER chain (leaf first) and key.
  #[cfg(feature = "tls")]
  pub fn der(
    certs: Vec<rustls::pki_types::CertificateDer<'static>>,
    key: rustls::pki_types::PrivateKeyDer<'static>,
  ) -> Self {
    Self::Der {
      certs: Arc::new(certs),
      key: Arc::new(key),
      client_auth: None,
    }
  }

  /// Certificate resolver for SNI multi-cert servers or hot reload (see [`ReloadableResolver`]).
  #[cfg(feature = "tls")]
  pub fn resolver(resolver: Arc<dyn rustls::server::ResolvesServerCert>) -> Self {
    Self::Resolver {
      resolver,
      client_auth: None,
    }
  }

  /// Replaces the mTLS policy of any variant.
  #[cfg(feature = "tls")]
  pub fn with_client_auth(mut self, auth: ClientAuth) -> Self {
    match &mut self {
      TlsCert::PemPaths { client_auth, .. }
      | TlsCert::Der { client_auth, .. }
      | TlsCert::Resolver { client_auth, .. } => *client_auth = Some(auth),
    }
    self
  }
}

/// Certificate resolver whose key can be swapped at runtime; keep the `Arc` passed to
/// [`TlsCert::resolver`] to reload. Each handshake costs one lock-free `Arc` clone.
///
/// # Example
///
/// ```rust,no_run
/// # #[cfg(feature = "tls")]
/// # async fn _example() -> anyhow::Result<()> {
/// use std::sync::Arc;
/// use tako_rs_server::{ReloadableResolver, TlsCert};
///
/// let resolver = Arc::new(ReloadableResolver::from_pem("cert.pem", "key.pem")?);
/// let cert = TlsCert::resolver(resolver.clone());
/// # #[cfg(not(feature = "compio"))]
/// let server = tako_rs_server::Server::builder().tls(cert).build();
/// # #[cfg(feature = "compio-tls")]
/// # let server = tako_rs_server::CompioServer::builder().tls(cert).build();
/// // Later, after a cert rotation:
/// resolver.reload_from_pem("cert.pem", "key.pem")?;
/// # Ok(())
/// # }
/// ```
#[cfg(feature = "tls")]
pub struct ReloadableResolver {
  current: arc_swap::ArcSwap<rustls::sign::CertifiedKey>,
}

#[cfg(feature = "tls")]
impl std::fmt::Debug for ReloadableResolver {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("ReloadableResolver").finish_non_exhaustive()
  }
}

#[cfg(feature = "tls")]
impl ReloadableResolver {
  /// Loads a certificate chain and key from PEM files.
  pub fn from_pem(cert_path: &str, key_path: &str) -> anyhow::Result<Self> {
    let ck = build_certified_key(cert_path, key_path)?;
    Ok(Self {
      current: arc_swap::ArcSwap::from_pointee(ck),
    })
  }

  /// Atomically swaps in a key loaded from PEM files; established sessions are unaffected and
  /// new handshakes use it.
  pub fn reload_from_pem(&self, cert_path: &str, key_path: &str) -> anyhow::Result<()> {
    let ck = build_certified_key(cert_path, key_path)?;
    self.current.store(Arc::new(ck));
    Ok(())
  }

  /// Atomically swaps in a prebuilt [`rustls::sign::CertifiedKey`].
  pub fn reload(&self, ck: rustls::sign::CertifiedKey) {
    self.current.store(Arc::new(ck));
  }
}

#[cfg(feature = "tls")]
impl rustls::server::ResolvesServerCert for ReloadableResolver {
  fn resolve(
    &self,
    _client_hello: rustls::server::ClientHello<'_>,
  ) -> Option<Arc<rustls::sign::CertifiedKey>> {
    Some(self.current.load_full())
  }
}

#[cfg(feature = "tls")]
fn build_certified_key(
  cert_path: &str,
  key_path: &str,
) -> anyhow::Result<rustls::sign::CertifiedKey> {
  let certs = tako_rs_core::tls::load_certs(cert_path)?;
  let key = tako_rs_core::tls::load_key(key_path)?;

  // Installs aws-lc-rs only when no provider exists yet; otherwise the earlier one, e.g. ring,
  // loads and signs with the key, which the one-time warning below reports.
  let we_installed = if rustls::crypto::CryptoProvider::get_default().is_none() {
    rustls::crypto::aws_lc_rs::default_provider()
      .install_default()
      .is_ok()
  } else {
    false
  };
  if !we_installed {
    static WARNED: std::sync::Once = std::sync::Once::new();
    WARNED.call_once(|| {
      tracing::warn!(
        "tako-server: a rustls CryptoProvider was already installed before \
         `build_certified_key` ran — Tako will use that provider for key \
         loading instead of installing aws-lc-rs. If signing behavior is \
         not what you expect (e.g. h3 installed `ring` first), pin the \
         provider at process startup with `rustls::crypto::aws_lc_rs::\
         default_provider().install_default()` BEFORE constructing the \
         server."
      );
    });
  }
  let provider = rustls::crypto::CryptoProvider::get_default().ok_or_else(|| {
    anyhow::anyhow!(
      "no rustls CryptoProvider installed — enable rustls's `aws_lc_rs` or `ring` feature"
    )
  })?;
  let signer = provider
    .key_provider
    .load_private_key(key)
    .map_err(|e| anyhow::anyhow!("failed to load signing key from '{key_path}': {e}"))?;
  Ok(rustls::sign::CertifiedKey::new(certs, signer))
}

/// Builds the rustls config the TLS spawn methods use, for the `serve_*_with_rustls_config`
/// functions. With `h3` in `alpn`, 0-RTT early data is disabled.
#[cfg(feature = "tls")]
pub fn build_rustls_server_config(
  cert: &TlsCert,
  alpn: Vec<Vec<u8>>,
) -> anyhow::Result<Arc<rustls::ServerConfig>> {
  use rustls::ServerConfig as RustlsServerConfig;

  // With `http3`, quinn compiles rustls's ring provider next to the default
  // aws-lc-rs one, and `ServerConfig::builder()` then refuses to choose.
  if rustls::crypto::CryptoProvider::get_default().is_none() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
  }
  let builder = RustlsServerConfig::builder();

  let client_auth = match cert {
    TlsCert::PemPaths { client_auth, .. }
    | TlsCert::Der { client_auth, .. }
    | TlsCert::Resolver { client_auth, .. } => client_auth.clone(),
  };

  let builder_with_auth = match client_auth {
    Some(ClientAuth::Optional(roots)) => {
      let verifier = rustls::server::WebPkiClientVerifier::builder(roots)
        .allow_unauthenticated()
        .build()
        .map_err(|e| anyhow::anyhow!("WebPkiClientVerifier build failed: {e}"))?;
      builder.with_client_cert_verifier(verifier)
    }
    Some(ClientAuth::Required(roots)) => {
      let verifier = rustls::server::WebPkiClientVerifier::builder(roots)
        .build()
        .map_err(|e| anyhow::anyhow!("WebPkiClientVerifier build failed: {e}"))?;
      builder.with_client_cert_verifier(verifier)
    }
    None => builder.with_no_client_auth(),
  };

  let mut config = match cert {
    TlsCert::PemPaths {
      cert_path,
      key_path,
      ..
    } => {
      let certs = tako_rs_core::tls::load_certs(cert_path)?;
      let key = tako_rs_core::tls::load_key(key_path)?;
      builder_with_auth
        .with_single_cert(certs, key)
        .map_err(|e| anyhow::anyhow!("rustls config build failed: {e}"))?
    }
    TlsCert::Der { certs, key, .. } => {
      let certs = certs.as_ref().clone();
      let key = key.as_ref().clone_key();
      builder_with_auth
        .with_single_cert(certs, key)
        .map_err(|e| anyhow::anyhow!("rustls config build failed: {e}"))?
    }
    TlsCert::Resolver { resolver, .. } => builder_with_auth.with_cert_resolver(resolver.clone()),
  };

  config.alpn_protocols = alpn;
  // Nothing protects requests from 0-RTT replay, so an h3 config never accepts early data.
  if config.alpn_protocols.iter().any(|p| p.as_slice() == b"h3") {
    config.max_early_data_size = 0;
  }
  Ok(Arc::new(config))
}
