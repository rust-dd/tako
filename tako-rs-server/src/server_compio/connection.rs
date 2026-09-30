//! Per-connection HTTP serving shared by every compio listener.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use compio::io::AsyncRead;
use compio::io::AsyncWrite;
use compio::io::util::Splittable;
use cyper_core::HyperStream;
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

/// Wire protocol served on every accepted connection.
#[derive(Clone, Copy)]
pub(crate) enum Protocol {
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
    }
  }
}

/// Remote endpoint of an accepted connection.
pub(crate) enum Peer {
  Tcp(SocketAddr),
  #[cfg(unix)]
  Unix(UnixPeerAddr),
}

impl Peer {
  fn annotate<B>(&self, req: &mut http::Request<B>, protocol: Protocol) {
    let extensions = req.extensions_mut();
    match self {
      Self::Tcp(addr) => {
        extensions.insert(*addr);
        extensions.insert(match protocol {
          Protocol::Http1 => ConnInfo::tcp(*addr),
          #[cfg(feature = "http2")]
          Protocol::H2c => ConnInfo::h2c(*addr),
        });
      }
      #[cfg(unix)]
      Self::Unix(peer) => {
        extensions.insert(ConnInfo::unix(peer.path.clone()));
        extensions.insert(peer.clone());
      }
    }
  }

  #[cfg(feature = "signals")]
  fn signal_label(&self) -> Option<String> {
    match self {
      Self::Tcp(addr) => Some(addr.to_string()),
      #[cfg(unix)]
      Self::Unix(_) => None,
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
}

impl From<&ServerConfig> for ConnectionSettings {
  fn from(config: &ServerConfig) -> Self {
    Self {
      keep_alive: config.keep_alive,
      header_read_timeout: config.header_read_timeout,
      #[cfg(feature = "http2")]
      h2: H2Settings::from(config),
    }
  }
}

pub(crate) async fn serve_connection<S>(
  stream: S,
  peer: Peer,
  router: Arc<Router>,
  protocol: Protocol,
  settings: ConnectionSettings,
  cancel: CancellationToken,
  permit: Option<OwnedSemaphorePermit>,
) where
  S: Splittable + 'static,
  S::ReadHalf: AsyncRead + Unpin,
  S::WriteHalf: AsyncWrite + Unpin,
{
  let _permit = permit;

  #[cfg(feature = "signals")]
  let label = peer.signal_label();
  #[cfg(feature = "signals")]
  if let Some(label) = &label {
    signal_tx::emit_connection_opened(label, false, None).await;
  }

  let io = HyperStream::new_plain(stream);
  let svc = service_fn(move |mut req| {
    peer.annotate(&mut req, protocol);
    let router = router.clone();
    async move {
      let response = router.dispatch(req.map(TakoBody::new)).await;
      Ok::<_, Infallible>(response)
    }
  });

  match protocol {
    Protocol::Http1 => {
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
    Protocol::H2c => {
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
