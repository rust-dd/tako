//! Accept loop shared by every compio HTTP listener: backoff on accept
//! errors, the connection cap, and bounded draining on shutdown.

use std::future::Future;
use std::io;
use std::sync::Arc;

use compio::io::AsyncRead;
use compio::io::AsyncWrite;
use compio::io::util::Splittable;
use futures_util::FutureExt;
use futures_util::StreamExt;
use futures_util::future::Either;
use futures_util::stream::FuturesUnordered;
use tako_rs_core::router::Router;
#[cfg(feature = "signals")]
use tako_rs_core::signals::transport as signal_tx;
use tako_rs_core::types::BoxError;
use tokio_util::sync::CancellationToken;

use super::connection::ConnectionSettings;
use super::connection::Peer;
use super::connection::Protocol;
use super::connection::serve_connection;
use crate::ServerConfig;

/// A compio listener the shared accept loop can drive.
pub(crate) trait Listener {
  type Stream: Splittable + 'static;

  /// Label for logs and lifecycle signals, e.g. `127.0.0.1:8080` or a path.
  fn describe(&self) -> io::Result<String>;

  /// Transport name reported with `server.started` and `server.stopped`.
  fn kind(&self) -> &'static str;

  fn next_connection(&self) -> impl Future<Output = io::Result<(Self::Stream, Peer)>>;

  /// Runs once after the last connection drained.
  fn finish(&self) {}
}

impl Listener for compio::net::TcpListener {
  type Stream = compio::net::TcpStream;

  fn describe(&self) -> io::Result<String> {
    Ok(self.local_addr()?.to_string())
  }

  fn kind(&self) -> &'static str {
    "tcp"
  }

  async fn next_connection(&self) -> io::Result<(Self::Stream, Peer)> {
    let (stream, addr) = self.accept().await?;
    Ok((stream, Peer::Tcp(addr)))
  }
}

pub(crate) async fn accept_loop<L>(
  listener: L,
  router: Router,
  signal: Option<impl Future<Output = ()>>,
  config: ServerConfig,
  protocol: Protocol,
) -> Result<(), BoxError>
where
  L: Listener,
  <L::Stream as Splittable>::ReadHalf: AsyncRead + Unpin,
  <L::Stream as Splittable>::WriteHalf: AsyncWrite + Unpin,
{
  #[cfg(feature = "tako-tracing")]
  tako_rs_core::tracing::init_tracing();

  let router = Arc::new(router);
  #[cfg(feature = "plugins")]
  router.setup_plugins_once()?;

  let label = listener.describe()?;

  #[cfg(feature = "signals")]
  signal_tx::emit_server_started(&label, listener.kind(), false).await;

  tracing::debug!(
    "Tako {} listening on {} {label}",
    protocol.name(),
    listener.kind()
  );

  let mut connections = FuturesUnordered::new();
  let drain_timeout = config.drain_timeout;
  let settings = ConnectionSettings::from(&config);

  let max_conn_semaphore = config
    .max_connections
    .map(|n| Arc::new(tokio::sync::Semaphore::new(n)));

  let mut accept_backoff = config.accept_backoff;

  let cancel = CancellationToken::new();
  let signal = signal.map(|s| Box::pin(s));
  let mut signal_fused = std::pin::pin!(async {
    if let Some(s) = signal {
      s.await;
    } else {
      std::future::pending::<()>().await;
    }
  });

  loop {
    let accept = std::pin::pin!(listener.next_connection());
    match futures_util::future::select(accept, signal_fused.as_mut()).await {
      Either::Left((result, _)) => {
        let (stream, peer) = match result {
          Ok(v) => {
            accept_backoff.reset();
            v
          }
          Err(err) => {
            tracing::warn!("compio accept failed: {err}; backing off");
            let d = accept_backoff.current_and_grow();

            let sleep = std::pin::pin!(compio::time::sleep(d));
            match futures_util::future::select(sleep, signal_fused.as_mut()).await {
              Either::Left(((), _)) => continue,
              Either::Right(_) => break,
            }
          }
        };

        let permit = if let Some(sem) = max_conn_semaphore.as_ref() {
          let acquire = std::pin::pin!(sem.clone().acquire_owned());
          match futures_util::future::select(acquire, signal_fused.as_mut()).await {
            Either::Left((Ok(p), _)) => Some(p),
            Either::Left((Err(_), _)) => continue,
            Either::Right(_) => break,
          }
        } else {
          None
        };

        connections.push(compio::runtime::spawn(serve_connection(
          stream,
          peer,
          router.clone(),
          protocol,
          settings,
          cancel.clone(),
          permit,
        )));
        while connections.next().now_or_never().flatten().is_some() {}
      }
      Either::Right(_) => {
        tracing::info!("Shutdown signal received, draining connections...");
        break;
      }
    }
  }

  cancel.cancel();
  if compio::time::timeout(drain_timeout, async {
    while connections.next().await.is_some() {}
  })
  .await
  .is_err()
  {
    tracing::warn!("drain timeout exceeded; cancelling remaining connections");
    for connection in connections {
      connection.cancel().await;
    }
  }

  listener.finish();
  tracing::info!("Server shut down gracefully");
  #[cfg(feature = "signals")]
  signal_tx::emit_server_stopped(&label, listener.kind(), false).await;
  Ok(())
}
