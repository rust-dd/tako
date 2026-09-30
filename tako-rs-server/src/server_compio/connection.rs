//! Per-connection HTTP serving shared by every compio listener.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use compio::io::AsyncRead;
use compio::io::AsyncWrite;
use compio::io::util::Splittable;
use cyper_core::HyperStream;
#[cfg(feature = "proxy-protocol")]
use futures_util::future::Either;
use hyper::server::conn::http1;
#[cfg(feature = "http2")]
use hyper::server::conn::http2;
use hyper::service::service_fn;
use tako_rs_core::body::TakoBody;
use tako_rs_core::conn_info::ConnInfo;
#[cfg(unix)]
use tako_rs_core::conn_info::UnixPeerAddr;
use tako_rs_core::router::Router;
use tako_rs_core::server_support::drive_connection;
#[cfg(feature = "signals")]
use tako_rs_core::signals::transport as signal_tx;
use tokio::sync::OwnedSemaphorePermit;
use tokio_util::sync::CancellationToken;

use crate::ServerConfig;
#[cfg(feature = "http2")]
use crate::compio_h2::H2Settings;
#[cfg(feature = "http2")]
use crate::compio_h2::ServiceSendWrapper;
#[cfg(feature = "proxy-protocol")]
use crate::proxy_protocol::ProxyHeader;

/// What a listener serves on every accepted connection.
#[derive(Clone, Copy)]
pub(crate) enum Protocol {
  Http1,
  #[cfg(feature = "http2")]
  H2c,
  /// HTTP/1 after a PROXY protocol v1/v2 header names the real client.
  #[cfg(feature = "proxy-protocol")]
  ProxyHttp1,
}

/// The HTTP flavour hyper speaks once any preamble has been read.
#[derive(Clone, Copy)]
enum Wire {
  Http1,
  #[cfg(feature = "http2")]
  H2c,
}

impl Protocol {
  pub(crate) fn name(self) -> &'static str {
    match self {
      Self::Http1 => "HTTP/1",
      #[cfg(feature = "http2")]
      Self::H2c => "h2c",
      #[cfg(feature = "proxy-protocol")]
      Self::ProxyHttp1 => "PROXY protocol HTTP/1",
    }
  }

  fn wire(self) -> Wire {
    match self {
      Self::Http1 => Wire::Http1,
      #[cfg(feature = "http2")]
      Self::H2c => Wire::H2c,
      #[cfg(feature = "proxy-protocol")]
      Self::ProxyHttp1 => Wire::Http1,
    }
  }
}

/// Remote endpoint of an accepted connection.
pub(crate) enum Peer {
  Tcp(SocketAddr),
  #[cfg(unix)]
  Unix(UnixPeerAddr),
  #[cfg(feature = "proxy-protocol")]
  Proxy(Box<ProxyHeader>),
}

impl Peer {
  fn annotate<B>(&self, req: &mut http::Request<B>, wire: Wire) {
    match self {
      Self::Tcp(addr) => {
        let extensions = req.extensions_mut();
        extensions.insert(*addr);
        extensions.insert(match wire {
          Wire::Http1 => ConnInfo::tcp(*addr),
          #[cfg(feature = "http2")]
          Wire::H2c => ConnInfo::h2c(*addr),
        });
      }
      #[cfg(unix)]
      Self::Unix(peer) => {
        let extensions = req.extensions_mut();
        extensions.insert(ConnInfo::unix(peer.path.clone()));
        extensions.insert(peer.clone());
      }
      #[cfg(feature = "proxy-protocol")]
      Self::Proxy(header) => crate::proxy_protocol::apply_to_request(req, header),
    }
  }

  #[cfg(feature = "signals")]
  fn signal_label(&self) -> Option<String> {
    match self {
      Self::Tcp(addr) => Some(addr.to_string()),
      #[cfg(unix)]
      Self::Unix(_) => None,
      #[cfg(feature = "proxy-protocol")]
      Self::Proxy(_) => None,
    }
  }
}

/// Per-connection limits copied out of [`ServerConfig`] once per server.
#[derive(Clone, Copy)]
pub(crate) struct ConnectionSettings {
  keep_alive: bool,
  header_read_timeout: Option<Duration>,
  #[cfg(feature = "http2")]
  h2: H2Settings,
  #[cfg(feature = "proxy-protocol")]
  proxy_read_timeout: Duration,
}

impl From<&ServerConfig> for ConnectionSettings {
  fn from(config: &ServerConfig) -> Self {
    Self {
      keep_alive: config.keep_alive,
      header_read_timeout: config.header_read_timeout,
      #[cfg(feature = "http2")]
      h2: H2Settings::from(config),
      #[cfg(feature = "proxy-protocol")]
      proxy_read_timeout: config.proxy_read_timeout,
    }
  }
}

pub(crate) async fn serve_connection<S>(
  #[cfg_attr(not(feature = "proxy-protocol"), allow(unused_mut))] mut stream: S,
  peer: Peer,
  router: Arc<Router>,
  protocol: Protocol,
  settings: ConnectionSettings,
  cancel: CancellationToken,
  permit: Option<OwnedSemaphorePermit>,
) where
  S: Splittable + AsyncRead + 'static,
  S::ReadHalf: AsyncRead + Unpin,
  S::WriteHalf: AsyncWrite + Unpin,
{
  let _permit = permit;

  #[cfg(feature = "proxy-protocol")]
  let peer = if matches!(protocol, Protocol::ProxyHttp1) {
    match read_proxy_header(&mut stream, settings.proxy_read_timeout, &cancel).await {
      Some(header) => Peer::Proxy(Box::new(header)),
      None => return,
    }
  } else {
    peer
  };

  #[cfg(feature = "signals")]
  let label = peer.signal_label();
  #[cfg(feature = "signals")]
  if let Some(label) = &label {
    signal_tx::emit_connection_opened(label, false, None).await;
  }

  let wire = protocol.wire();
  let io = HyperStream::new_plain(stream);
  let svc = service_fn(move |mut req| {
    peer.annotate(&mut req, wire);
    let router = router.clone();
    async move {
      let response = router.dispatch(req.map(TakoBody::new)).await;
      Ok::<_, Infallible>(response)
    }
  });

  match wire {
    Wire::Http1 => {
      let mut http = http1::Builder::new();
      http.keep_alive(settings.keep_alive);
      http
        .timer(cyper_core::CompioTimer)
        .header_read_timeout(settings.header_read_timeout);
      let conn = http.serve_connection(io, svc).with_upgrades();

      if let Err(err) = drive_connection(
        conn,
        cancel.cancelled(),
        http1::UpgradeableConnection::graceful_shutdown,
      )
      .await
      {
        if err.is_incomplete_message() {
          tracing::debug!("client disconnected mid-message: {err}");
        } else {
          tracing::error!("Error serving connection: {err}");
        }
      }
    }
    #[cfg(feature = "http2")]
    Wire::H2c => {
      let conn = settings
        .h2
        .builder()
        .serve_connection(io, ServiceSendWrapper::new(svc));
      if let Err(err) = drive_connection(
        conn,
        cancel.cancelled(),
        http2::Connection::graceful_shutdown,
      )
      .await
      {
        tracing::warn!("h2c connection error: {err}");
      }
    }
  }

  #[cfg(feature = "signals")]
  if let Some(label) = &label {
    signal_tx::emit_connection_closed(label, false, None).await;
  }
}

/// Reads the PROXY header within `timeout`; `None` drops the connection.
#[cfg(feature = "proxy-protocol")]
async fn read_proxy_header<S: AsyncRead>(
  stream: &mut S,
  timeout: Duration,
  cancel: &CancellationToken,
) -> Option<ProxyHeader> {
  let cancelled = std::pin::pin!(cancel.cancelled());
  let read = std::pin::pin!(compio::time::timeout(
    timeout,
    crate::proxy_protocol::read_proxy_protocol(stream),
  ));
  match futures_util::future::select(cancelled, read).await {
    Either::Left(_) => None,
    Either::Right((Ok(Ok(header)), _)) => Some(header),
    Either::Right((Ok(Err(e)), _)) => {
      tracing::warn!("Failed to parse PROXY protocol: {e}");
      None
    }
    Either::Right((Err(_), _)) => {
      tracing::warn!("PROXY protocol read deadline ({timeout:?}) elapsed; dropping connection");
      None
    }
  }
}
