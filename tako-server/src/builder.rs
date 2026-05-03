//! Unified [`Server`] / [`CompioServer`] builder fronting every Tako transport.
//!
//! The direct `serve_*` / `serve_*_with_shutdown` / `*_with_config` functions
//! still exist and keep working. This module is an additive convenience layer:
//! pick a transport via `spawn_*`, hand it a [`crate::ServerConfig`], and get
//! back a [`ServerHandle`] that owns a shutdown trigger.
//!
//! The handle itself is runtime-agnostic — both [`Server`] (tokio) and
//! [`CompioServer`] (cfg `compio`) return the same [`ServerHandle`] type.
//! Internally each `spawn_*` wraps the underlying `serve_*` future so that
//! when it returns, a `done` [`Notify`] is signalled. [`ServerHandle::join`]
//! awaits that notify; [`ServerHandle::shutdown`] triggers the shutdown
//! signal and then awaits the same `done`.
//!
//! No additional allocation or atomic swap is introduced on the per-connection
//! / per-request hot path — the spawn wrapper is a single async block over the
//! underlying `serve_*_with_shutdown_and_config` call.

use std::future::Future;
#[cfg(not(feature = "compio"))]
use std::path::PathBuf;
#[cfg(not(feature = "compio"))]
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tako_core::router::Router;
#[cfg(not(feature = "compio"))]
use tokio::net::TcpListener;
use tokio::sync::Notify;

use crate::ServerConfig;

/// Background-task handle returned by every `spawn_*` method.
///
/// Drop semantics: dropping the handle does **not** stop the server. Call
/// [`ServerHandle::shutdown`] (or [`ServerHandle::trigger`] + `.join().await`)
/// so the drain logic in the underlying `serve_*_with_shutdown` runs.
///
/// Runtime-agnostic — the `done` signal is fired by an `async` wrapper around
/// the underlying `serve_*` future, so the same `ServerHandle` works whether
/// the spawned task lives on the tokio runtime or the compio runtime.
pub struct ServerHandle {
  shutdown: Arc<Notify>,
  done: Arc<Notify>,
  drain_timeout: Duration,
}

impl ServerHandle {
  /// Trigger graceful shutdown without awaiting completion.
  pub fn trigger(&self) {
    self.shutdown.notify_waiters();
  }

  /// Await the spawned task's completion (without triggering shutdown).
  ///
  /// Returns when the underlying `serve_*` future resolves — typically
  /// because [`ServerHandle::trigger`] / [`ServerHandle::shutdown`] was called
  /// or because the listener errored fatally.
  pub async fn join(&self) {
    self.done.notified().await;
  }

  /// Trigger graceful shutdown and await the drain.
  ///
  /// The `_timeout` argument is kept for API symmetry with the original
  /// builder; the actual drain bound is the `drain_timeout` on the
  /// [`ServerConfig`] that was handed to the builder, enforced inside
  /// `serve_*_with_shutdown`.
  pub async fn shutdown(self, _timeout: Duration) {
    self.shutdown.notify_waiters();
    self.done.notified().await;
  }

  /// Returns the drain timeout the underlying `serve_*` will honor.
  #[inline]
  pub fn drain_timeout(&self) -> Duration {
    self.drain_timeout
  }
}

impl std::fmt::Debug for ServerHandle {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("ServerHandle")
      .field("drain_timeout", &self.drain_timeout)
      .finish_non_exhaustive()
  }
}

/// Convenience: await `signal_a` *or* `signal_b`, whichever fires first.
pub async fn either<A, B>(a: A, b: B)
where
  A: Future<Output = ()>,
  B: Future<Output = ()>,
{
  use futures_util::future::Either;
  let a = std::pin::pin!(a);
  let b = std::pin::pin!(b);
  match futures_util::future::select(a, b).await {
    Either::Left(_) | Either::Right(_) => {}
  }
}

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
/// use tako_server::{ReloadableResolver, Server, TlsCert};
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
  let certs = tako_core::tls::load_certs(cert_path)?;
  let key = tako_core::tls::load_key(key_path)?;

  // Use whatever rustls CryptoProvider is installed — server_h3 / webtransport
  // install `ring` on first use; pure-TLS apps may not have installed any yet.
  // Opportunistically install rustls's default backend (`aws-lc-rs`) if the
  // global slot is still empty, so callers don't have to wire it themselves.
  if rustls::crypto::CryptoProvider::get_default().is_none() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
  }
  let provider = rustls::crypto::CryptoProvider::get_default().ok_or_else(|| {
    anyhow::anyhow!(
      "no rustls CryptoProvider installed — enable rustls's `aws_lc_rs` or `ring` feature"
    )
  })?;
  let signer = provider
    .key_provider
    .load_private_key(key)
    .map_err(|e| anyhow::anyhow!("failed to load signing key from '{}': {}", key_path, e))?;
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
      let certs = tako_core::tls::load_certs(cert_path)?;
      let key = tako_core::tls::load_key(key_path)?;
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
  Ok(Arc::new(config))
}

/// Fluent constructor for the tokio-runtime [`Server`].
#[cfg(not(feature = "compio"))]
#[derive(Debug, Default, Clone)]
pub struct ServerBuilder {
  config: ServerConfig,
  tls: Option<TlsCert>,
}

#[cfg(not(feature = "compio"))]
impl ServerBuilder {
  /// Override the [`ServerConfig`] (drain timeout, h2 caps, max_connections, …).
  #[must_use]
  pub fn config(mut self, config: ServerConfig) -> Self {
    self.config = config;
    self
  }

  /// Attach TLS material so [`Server::spawn_tls`] / [`Server::spawn_h3`] become usable.
  #[must_use]
  pub fn tls(mut self, cert: TlsCert) -> Self {
    self.tls = Some(cert);
    self
  }

  /// Finalize and produce the [`Server`].
  pub fn build(self) -> Server {
    Server {
      config: self.config,
      tls: self.tls,
    }
  }
}

/// Tokio-runtime server entry point. Construct with [`Server::builder`].
#[cfg(not(feature = "compio"))]
#[derive(Debug, Clone)]
pub struct Server {
  config: ServerConfig,
  // Read only by the `tls` / `http3` cfg-gated spawn methods; the field is
  // always present so the builder API surface stays stable across feature
  // combinations.
  #[cfg_attr(not(any(feature = "tls", feature = "http3")), allow(dead_code))]
  tls: Option<TlsCert>,
}

#[cfg(not(feature = "compio"))]
impl Server {
  /// Start a fresh fluent builder.
  #[must_use]
  pub fn builder() -> ServerBuilder {
    ServerBuilder::default()
  }

  /// Borrow the underlying [`ServerConfig`].
  #[inline]
  pub fn config(&self) -> &ServerConfig {
    &self.config
  }

  // ── HTTP family (router-driven) ──

  /// Spawn a plain HTTP/1 server.
  pub fn spawn_http(&self, listener: TcpListener, router: Router) -> ServerHandle {
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    let config = self.config.clone();
    spawn_done(handle.done.clone(), async move {
      crate::server::serve_with_shutdown_and_config(listener, router, shutdown_fut, config).await;
    });
    handle
  }

  /// Spawn an h2c (HTTP/2 cleartext, prior knowledge) server.
  #[cfg(feature = "http2")]
  pub fn spawn_h2c(&self, listener: TcpListener, router: Router) -> ServerHandle {
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    let config = self.config.clone();
    spawn_done(handle.done.clone(), async move {
      crate::server_h2c::serve_h2c_with_shutdown_and_config(listener, router, shutdown_fut, config)
        .await;
    });
    handle
  }

  /// Spawn a TLS server. Requires that the builder was given a [`TlsCert`].
  ///
  /// Dispatches on the [`TlsCert`] variant: `PemPaths` keeps the legacy
  /// path-loaded fast path; `Der` and `Resolver` (and any client-auth/mTLS
  /// configuration) go through [`crate::build_rustls_server_config`].
  #[cfg(feature = "tls")]
  pub fn spawn_tls(&self, listener: TcpListener, router: Router) -> ServerHandle {
    let tls = self
      .tls
      .clone()
      .expect("Server::spawn_tls requires a TlsCert (use builder().tls(...))");
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    let config = self.config.clone();
    let alpn = tls_alpn_for_tcp();
    spawn_done(handle.done.clone(), async move {
      // Plain `PemPaths` without mTLS keeps the no-overhead path-based loader;
      // every other variant goes through the rustls-config helper.
      if let TlsCert::PemPaths {
        cert_path,
        key_path,
        client_auth: None,
      } = &tls
      {
        crate::server_tls::serve_tls_with_shutdown_and_config(
          listener,
          router,
          Some(cert_path.as_str()),
          Some(key_path.as_str()),
          shutdown_fut,
          config,
        )
        .await;
        return;
      }
      let rustls_cfg = match build_rustls_server_config(&tls, alpn) {
        Ok(c) => c,
        Err(e) => {
          tracing::error!("Server::spawn_tls: failed to build rustls config: {e}");
          return;
        }
      };
      crate::server_tls::serve_tls_with_rustls_config_and_shutdown(
        listener,
        router,
        rustls_cfg,
        shutdown_fut,
        config,
      )
      .await;
    });
    handle
  }

  /// Spawn an HTTP/3 (QUIC) server. Binds to `addr` internally; takes TLS
  /// from the builder. Requires the `http3` feature.
  #[cfg(feature = "http3")]
  pub fn spawn_h3(&self, addr: impl Into<String>, router: Router) -> ServerHandle {
    let tls = self
      .tls
      .clone()
      .expect("Server::spawn_h3 requires a TlsCert (use builder().tls(...))");
    let addr = addr.into();
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    let config = self.config.clone();
    spawn_done(handle.done.clone(), async move {
      if let TlsCert::PemPaths {
        cert_path,
        key_path,
        client_auth: None,
      } = &tls
      {
        crate::server_h3::serve_h3_with_shutdown_and_config(
          router,
          &addr,
          Some(cert_path.as_str()),
          Some(key_path.as_str()),
          shutdown_fut,
          config,
        )
        .await;
        return;
      }
      let rustls_cfg = match build_rustls_server_config(&tls, vec![b"h3".to_vec()]) {
        Ok(c) => c,
        Err(e) => {
          tracing::error!("Server::spawn_h3: failed to build rustls config: {e}");
          return;
        }
      };
      crate::server_h3::serve_h3_with_rustls_config_and_shutdown(
        router,
        &addr,
        rustls_cfg,
        shutdown_fut,
        config,
      )
      .await;
    });
    handle
  }

  /// Spawn an HTTP-over-Unix-socket server.
  #[cfg(unix)]
  pub fn spawn_unix_http(&self, path: impl Into<PathBuf>, router: Router) -> ServerHandle {
    let path = path.into();
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    let config = self.config.clone();
    spawn_done(handle.done.clone(), async move {
      crate::server_unix::serve_unix_http_with_shutdown_and_config(
        path,
        router,
        shutdown_fut,
        config,
      )
      .await;
    });
    handle
  }

  /// Spawn an HTTP server bound to a Linux vsock `(cid, port)` pair. Requires
  /// the `vsock` feature and Linux.
  #[cfg(all(target_os = "linux", feature = "vsock"))]
  pub fn spawn_vsock_http(&self, cid: u32, port: u32, router: Router) -> ServerHandle {
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    let config = self.config.clone();
    spawn_done(handle.done.clone(), async move {
      crate::server_vsock::serve_vsock_http_with_shutdown_and_config(
        cid,
        port,
        router,
        shutdown_fut,
        config,
      )
      .await;
    });
    handle
  }

  /// Spawn an HTTP server fronted by PROXY-protocol parsing.
  pub fn spawn_proxy_protocol(&self, listener: TcpListener, router: Router) -> ServerHandle {
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    let config = self.config.clone();
    spawn_done(handle.done.clone(), async move {
      crate::proxy_protocol::serve_http_with_proxy_protocol_shutdown_and_config(
        listener,
        router,
        shutdown_fut,
        config,
      )
      .await;
    });
    handle
  }

  // ── Raw transports (handler-driven, no router) ──

  /// Spawn a raw TCP server. The handler receives each accepted stream.
  pub fn spawn_tcp_raw<F>(&self, addr: impl Into<String>, handler: F) -> ServerHandle
  where
    F: Fn(
        tokio::net::TcpStream,
        std::net::SocketAddr,
      ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>
      + Send
      + Sync
      + 'static,
  {
    let addr = addr.into();
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    spawn_done(handle.done.clone(), async move {
      if let Err(e) = crate::server_tcp::serve_tcp_with_shutdown(&addr, handler, shutdown_fut).await
      {
        tracing::error!("raw TCP server error: {e}");
      }
    });
    handle
  }

  /// Spawn a raw UDP server. The handler receives each datagram.
  pub fn spawn_udp_raw<F>(&self, addr: impl Into<String>, handler: F) -> ServerHandle
  where
    F: Fn(
        Vec<u8>,
        std::net::SocketAddr,
        Arc<tokio::net::UdpSocket>,
      ) -> Pin<Box<dyn Future<Output = ()> + Send>>
      + Send
      + Sync
      + 'static,
  {
    let addr = addr.into();
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    spawn_done(handle.done.clone(), async move {
      if let Err(e) = crate::server_udp::serve_udp_with_shutdown(&addr, handler, shutdown_fut).await
      {
        tracing::error!("raw UDP server error: {e}");
      }
    });
    handle
  }
}

/// Fluent constructor for the compio-runtime [`CompioServer`].
#[cfg(feature = "compio")]
#[derive(Debug, Default, Clone)]
pub struct CompioServerBuilder {
  config: ServerConfig,
  tls: Option<TlsCert>,
}

#[cfg(feature = "compio")]
impl CompioServerBuilder {
  /// Override the [`ServerConfig`].
  #[must_use]
  pub fn config(mut self, config: ServerConfig) -> Self {
    self.config = config;
    self
  }

  /// Attach TLS material so [`CompioServer::spawn_tls`] becomes usable.
  #[must_use]
  pub fn tls(mut self, cert: TlsCert) -> Self {
    self.tls = Some(cert);
    self
  }

  /// Finalize and produce the [`CompioServer`].
  pub fn build(self) -> CompioServer {
    CompioServer {
      config: self.config,
      tls: self.tls,
    }
  }
}

/// Compio-runtime server entry point. Construct with [`CompioServer::builder`].
///
/// Mirrors the tokio [`Server`] API but drives the compio runtime — io_uring
/// on Linux, IOCP on Windows, kqueue on macOS — under the hood.
#[cfg(feature = "compio")]
#[derive(Debug, Clone)]
pub struct CompioServer {
  config: ServerConfig,
  tls: Option<TlsCert>,
}

#[cfg(feature = "compio")]
impl CompioServer {
  /// Start a fresh fluent builder.
  #[must_use]
  pub fn builder() -> CompioServerBuilder {
    CompioServerBuilder::default()
  }

  /// Borrow the underlying [`ServerConfig`].
  #[inline]
  pub fn config(&self) -> &ServerConfig {
    &self.config
  }

  /// Spawn a compio HTTP/1 server.
  pub fn spawn_http(&self, listener: compio::net::TcpListener, router: Router) -> ServerHandle {
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    let config = self.config.clone();
    spawn_done_compio(handle.done.clone(), async move {
      crate::server_compio::serve_with_shutdown_and_config(listener, router, shutdown_fut, config)
        .await;
    });
    handle
  }

  /// Spawn a compio TLS server.
  #[cfg(feature = "compio-tls")]
  pub fn spawn_tls(&self, listener: compio::net::TcpListener, router: Router) -> ServerHandle {
    let tls = self
      .tls
      .clone()
      .expect("CompioServer::spawn_tls requires a TlsCert (use builder().tls(...))");
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    let config = self.config.clone();
    let alpn = tls_alpn_for_tcp();
    spawn_done_compio(handle.done.clone(), async move {
      if let TlsCert::PemPaths {
        cert_path,
        key_path,
        client_auth: None,
      } = &tls
      {
        crate::server_tls_compio::serve_tls_with_shutdown_and_config(
          listener,
          router,
          Some(cert_path.as_str()),
          Some(key_path.as_str()),
          shutdown_fut,
          config,
        )
        .await;
        return;
      }
      let rustls_cfg = match build_rustls_server_config(&tls, alpn) {
        Ok(c) => c,
        Err(e) => {
          tracing::error!("CompioServer::spawn_tls: failed to build rustls config: {e}");
          return;
        }
      };
      crate::server_tls_compio::serve_tls_with_rustls_config_and_shutdown(
        listener,
        router,
        rustls_cfg,
        shutdown_fut,
        config,
      )
      .await;
    });
    handle
  }
}

/// ALPN list used by TCP-based TLS spawn paths. Mirrors the per-feature
/// negotiation already done in `server_tls{,_compio}::run`.
#[cfg(feature = "tls")]
#[inline]
fn tls_alpn_for_tcp() -> Vec<Vec<u8>> {
  #[cfg(feature = "http2")]
  {
    vec![b"h2".to_vec(), b"http/1.1".to_vec()]
  }
  #[cfg(not(feature = "http2"))]
  {
    vec![b"http/1.1".to_vec()]
  }
}

fn make_handle(
  drain_timeout: Duration,
) -> (ServerHandle, impl Future<Output = ()> + Send + 'static) {
  let shutdown = Arc::new(Notify::new());
  let done = Arc::new(Notify::new());
  let shutdown_for_task = shutdown.clone();
  // Hold the Arc inside the future so it stays alive across the spawn move,
  // and call notified() *inside* an async block so the same NotifyFuture is
  // polled across wakeups (a fresh notified() per poll loses the racing
  // notify_waiters() and deadlocks).
  let fut = async move {
    shutdown_for_task.notified().await;
  };
  (
    ServerHandle {
      shutdown,
      done,
      drain_timeout,
    },
    fut,
  )
}

#[cfg(not(feature = "compio"))]
fn spawn_done<F>(done: Arc<Notify>, fut: F)
where
  F: Future<Output = ()> + Send + 'static,
{
  tokio::spawn(async move {
    fut.await;
    done.notify_waiters();
  });
}

#[cfg(feature = "compio")]
fn spawn_done_compio<F>(done: Arc<Notify>, fut: F)
where
  F: Future<Output = ()> + 'static,
{
  compio::runtime::spawn(async move {
    fut.await;
    done.notify_waiters();
  })
  .detach();
}
