// Used by both the TLS variants of `TlsCert` (any combination of compio+tls
// or non-compio+tls) and the rustls config builders below — every reference in
// this module is gated on the `tls` feature.
#[cfg(feature = "tls")]
use std::sync::Arc;

/// Client-authentication policy applied to a TLS server.
///
/// Both variants carry the trusted [`rustls::RootCertStore`] used to validate
/// the client-presented chain. `Optional` allows clients without a cert to
/// proceed (the application can later inspect the peer certs); `Required`
/// terminates handshakes that omit a cert.
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

/// Optional TLS material the builder can attach to a TLS-mode server.
///
/// Variants:
/// - [`TlsCert::PemPaths`] — load cert and key from disk on every spawn.
/// - [`TlsCert::Der`] — pre-loaded DER cert chain + key.
/// - [`TlsCert::Resolver`] — user-supplied [`rustls::server::ResolvesServerCert`]
///   for SNI multi-cert serving or hot-reloadable certificates (see
///   [`ReloadableResolver`]).
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
  /// Pre-loaded DER cert chain + key. Useful when certs come from secret
  /// storage rather than the filesystem.
  #[cfg(feature = "tls")]
  Der {
    /// DER-encoded certificate chain (leaf first).
    certs: Arc<Vec<rustls::pki_types::CertificateDer<'static>>>,
    /// DER-encoded private key.
    key: Arc<rustls::pki_types::PrivateKeyDer<'static>>,
    /// Optional mTLS policy.
    client_auth: Option<ClientAuth>,
  },
  /// User-supplied certificate resolver. The most flexible variant — drives
  /// SNI multi-cert serving, hot reload (see [`ReloadableResolver`]), and any
  /// custom logic that picks a cert per client-hello.
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
  /// Construct from filesystem paths (PEM cert + PEM key).
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

  /// Construct from pre-loaded DER cert chain + key.
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

  /// Construct from a user-supplied certificate resolver. This is the entry
  /// point for SNI multi-cert servers and hot-reload (see [`ReloadableResolver`]).
  #[cfg(feature = "tls")]
  pub fn resolver(resolver: Arc<dyn rustls::server::ResolvesServerCert>) -> Self {
    Self::Resolver {
      resolver,
      client_auth: None,
    }
  }

  /// Returns a clone of the resolver (or no-op for static cert variants).
  ///
  /// Useful when the caller wants to swap the live cert at runtime — they pass
  /// in a [`ReloadableResolver`] via [`TlsCert::resolver`] and keep the `Arc`
  /// for later `.reload_*()` calls.
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

/// A `ResolvesServerCert` whose backing [`rustls::sign::CertifiedKey`] can be
/// swapped at runtime via [`ReloadableResolver::reload_from_pem`].
///
/// Backed by [`arc_swap::ArcSwap`], the swap is atomic and lock-free on the
/// hot path (one `Arc` clone per TLS handshake). Use it via
/// [`TlsCert::resolver`] and keep the returned `Arc` so callers can trigger
/// reloads from anywhere (file watcher, signal handler, admin endpoint, …).
///
/// # Example
///
/// ```rust,no_run
/// # #[cfg(feature = "tls")]
/// # async fn _example() -> anyhow::Result<()> {
/// use std::sync::Arc;
/// use tako_rs_server::{ReloadableResolver, Server, TlsCert};
///
/// let resolver = Arc::new(ReloadableResolver::from_pem("cert.pem", "key.pem")?);
/// let cert = TlsCert::resolver(resolver.clone());
/// let server = Server::builder().tls(cert).build();
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
  /// Construct from on-disk PEM files.
  pub fn from_pem(cert_path: &str, key_path: &str) -> anyhow::Result<Self> {
    let ck = build_certified_key(cert_path, key_path)?;
    Ok(Self {
      current: arc_swap::ArcSwap::from_pointee(ck),
    })
  }

  /// Atomically swap to a new cert + key loaded from the given PEM files.
  ///
  /// Hot-path TLS handshakes pick up the new cert on the next `resolve` call
  /// without dropping any in-flight session.
  pub fn reload_from_pem(&self, cert_path: &str, key_path: &str) -> anyhow::Result<()> {
    let ck = build_certified_key(cert_path, key_path)?;
    self.current.store(Arc::new(ck));
    Ok(())
  }

  /// Atomically swap to a pre-built [`rustls::sign::CertifiedKey`].
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

  // Use whatever rustls CryptoProvider is installed — server_h3 / webtransport
  // install `ring` on first use; pure-TLS apps may not have installed any yet.
  // Opportunistically install rustls's default backend (`aws-lc-rs`) if the
  // global slot is still empty, so callers don't have to wire it themselves.
  //
  // SRV-08: if a provider was ALREADY installed by another part of the
  // process (commonly `ring` via h3/webtransport bootstrap), this code uses
  // it as-is and `load_private_key` runs against that backend. Cross-
  // provider key loading is supported in principle, but signature output
  // depends on which backend signs — operators surprised by behavior diff
  // need to know we did not install aws-lc-rs in that case.
  let we_installed = if rustls::crypto::CryptoProvider::get_default().is_none() {
    rustls::crypto::aws_lc_rs::default_provider()
      .install_default()
      .is_ok()
  } else {
    false
  };
  if !we_installed {
    // Fire a one-shot warning so it shows up once in the log instead of
    // once per certified-key build (which could be thousands per process
    // in tests / hot-reload). Static `Once` keeps this lock-free after
    // the first call.
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

/// Build an `Arc<rustls::ServerConfig>` from a [`TlsCert`] and the desired
/// ALPN protocol list.
///
/// Internal helper used by every `Server::spawn_*` TLS-mode method. Exposed
/// so embedders can build the rustls config the same way Tako does and pass
/// it to the lower-level `serve_*_with_rustls_config_*` entrypoints.
#[cfg(feature = "tls")]
pub fn build_rustls_server_config(
  cert: &TlsCert,
  alpn: Vec<Vec<u8>>,
) -> anyhow::Result<Arc<rustls::ServerConfig>> {
  use rustls::ServerConfig as RustlsServerConfig;

  let builder = RustlsServerConfig::builder();

  // Resolve the client-auth verifier first. `with_no_client_auth` and
  // `with_client_cert_verifier` produce the same `ConfigBuilder<...>` next
  // step, so we can branch cleanly here.
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
  // Defense in depth against H3 0-RTT replay: when ALPN advertises h3 the
  // resulting config will end up driving a quinn endpoint, and Tako has no
  // replay cache on the request path. `server_h3::run_with_rustls_config`
  // also clears this defensively, but doing it at construction prevents the
  // window where the unprotected config exists in caller memory before being
  // handed to `serve_h3_*`.
  if config.alpn_protocols.iter().any(|p| p.as_slice() == b"h3") {
    config.max_early_data_size = 0;
  }
  Ok(Arc::new(config))
}
