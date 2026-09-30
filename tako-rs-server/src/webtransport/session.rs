use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use bytes::Bytes;
use futures_util::FutureExt;
use futures_util::future::Either;
use futures_util::future::FusedFuture;
use tako_rs_core::router::Router;
use tako_rs_core::types::BoxError;
use tokio_util::sync::CancellationToken;

use super::BidiStream;
use super::RecvStream;
use super::SendStream;
use super::SessionId;
use super::WebTransportClose;
use super::capsule;
use super::driver::ConnectSend;
use super::driver::Driver;
use crate::h3_common::connection::QuicConnection;
use crate::h3_common::request::handle_request;

/// An established WebTransport session.
///
/// Clone it to use the session from several tasks; on compio the clones stay
/// on the connection's thread. Once the session is closed — by the client,
/// by [`close`](Self::close), or by server shutdown — the accept and read
/// methods return `Ok(None)`. When the handler returns, the server ends the
/// session and closes its connection.
#[derive(Clone)]
pub struct WebTransportSession {
  inner: Shared<Inner>,
}

/// Compio sessions never leave their thread, so they skip atomic counting.
#[cfg(not(feature = "compio"))]
type Shared<T> = Arc<T>;
#[cfg(feature = "compio")]
type Shared<T> = std::rc::Rc<T>;

struct Inner {
  driver: Driver,
  /// `None` once the session has been closed from this side.
  connect: tokio::sync::Mutex<Option<ConnectSend>>,
  closed: CancellationToken,
  close: OnceLock<WebTransportClose>,
  router: Arc<Router>,
  remote_addr: SocketAddr,
}

impl WebTransportSession {
  /// Accepts the session and runs `handler` until it returns, or until
  /// `grace` after the client closed the session or the server started to
  /// shut down.
  pub(crate) async fn run<F: Future<Output = ()>>(
    stream: crate::h3_common::sessions::ServerStream,
    conn: h3::server::Connection<QuicConnection, Bytes>,
    handler: impl FnOnce(Self) -> F,
    router: Arc<Router>,
    remote_addr: SocketAddr,
    shutdown: &CancellationToken,
    grace: Duration,
  ) {
    let (driver, send, recv) = match Driver::accept(stream, conn).await {
      Ok(accepted) => accepted,
      Err(e) => {
        tracing::debug!("WebTransport session failed to start: {e}");
        return;
      }
    };
    let inner = Shared::new(Inner {
      driver,
      connect: tokio::sync::Mutex::new(Some(send)),
      closed: CancellationToken::new(),
      close: OnceLock::new(),
      router,
      remote_addr,
    });

    let mut watch = std::pin::pin!(capsule::watch(recv).fuse());
    {
      let handler = std::pin::pin!(handler(Self {
        inner: inner.clone()
      }));
      let ended = std::pin::pin!(async {
        let shutdown = std::pin::pin!(shutdown.cancelled());
        if let Either::Left((close, _)) =
          futures_util::future::select(watch.as_mut(), shutdown).await
        {
          let _ = inner.close.set(close);
        }
        inner.closed.cancel();
        sleep(grace).await;
      });
      futures_util::future::select(handler, ended).await;
    }

    // Finishing the CONNECT stream is a clean close with code 0. Dropping the
    // connection would discard a close capsule still in flight, so wait for
    // the client to end its side first.
    let close = async {
      if let Some(mut connect) = inner.connect.lock().await.take() {
        let _ = connect.finish().await;
      }
      if !watch.is_terminated() {
        watch.as_mut().await;
      }
    };
    let _ = futures_util::future::select(std::pin::pin!(close), std::pin::pin!(sleep(grace))).await;
  }

  /// The id of the session, which is the stream id of its CONNECT request.
  pub fn session_id(&self) -> SessionId {
    self.inner.driver.id()
  }

  /// The client's address.
  pub fn remote_address(&self) -> SocketAddr {
    self.inner.remote_addr
  }

  /// Waits for the next bidirectional stream the client opens.
  ///
  /// HTTP/3 requests that arrive on the same connection are served by the
  /// router meanwhile.
  pub async fn accept_bi(&self) -> Result<Option<BidiStream>, BoxError> {
    self
      .until_closed(async {
        loop {
          let accepted = self
            .inner
            .driver
            .accept_bi(|req, stream| self.serve_request(req, stream))
            .await?;
          match accepted {
            None => return Ok(None),
            Some((id, stream)) if id == self.session_id() => return Ok(Some(stream)),
            Some(_) => {}
          }
        }
      })
      .await
  }

  /// Waits for the next unidirectional stream the client opens.
  pub async fn accept_uni(&self) -> Result<Option<RecvStream>, BoxError> {
    self
      .until_closed(async {
        loop {
          let (id, stream) = self.inner.driver.accept_uni().await?;
          if id == self.session_id() {
            return Ok(Some(stream));
          }
        }
      })
      .await
  }

  /// Opens a bidirectional stream to the client.
  pub async fn open_bi(&self) -> Result<BidiStream, BoxError> {
    Ok(self.inner.driver.open_bi().await?)
  }

  /// Opens a unidirectional stream to the client.
  pub async fn open_uni(&self) -> Result<SendStream, BoxError> {
    Ok(self.inner.driver.open_uni().await?)
  }

  /// Waits for the next datagram of this session.
  pub async fn read_datagram(&self) -> Result<Option<Bytes>, BoxError> {
    self
      .until_closed(async {
        let mut reader = self.inner.driver.datagram_reader();
        loop {
          let datagram = reader.read_datagram().await?;
          if SessionId::from(datagram.stream_id()) == self.session_id() {
            return Ok(Some(datagram.into_payload()));
          }
        }
      })
      .await
  }

  /// Sends an unreliable datagram; it may be dropped or arrive out of order.
  pub fn send_datagram(&self, payload: Bytes) -> Result<(), BoxError> {
    self.inner.driver.datagram_sender().send_datagram(payload)?;
    Ok(())
  }

  /// Closes the session with an application error code and a reason, which
  /// the browser reports from `WebTransport.closed`. The reason is trimmed to
  /// 1024 bytes.
  pub async fn close(&self, code: u32, reason: &str) -> Result<(), BoxError> {
    let (capsule, reason) = capsule::encode_close(code, reason);
    let connect = self.inner.connect.lock().await.take();
    let _ = self.inner.close.set(WebTransportClose { code, reason });
    self.inner.closed.cancel();
    if let Some(mut connect) = connect {
      connect.send_data(capsule).await?;
      connect.finish().await?;
    }
    Ok(())
  }

  /// Resolves once the session is closed, with the code and reason of
  /// whichever side closed it first.
  pub async fn closed(&self) -> WebTransportClose {
    self.inner.closed.cancelled().await;
    self.inner.close.get().cloned().unwrap_or_default()
  }

  async fn until_closed<T>(
    &self,
    operation: impl Future<Output = Result<Option<T>, BoxError>>,
  ) -> Result<Option<T>, BoxError> {
    let operation = std::pin::pin!(operation);
    let closed = std::pin::pin!(self.inner.closed.cancelled());
    match futures_util::future::select(operation, closed).await {
      Either::Left((result, _)) => result,
      Either::Right(_) => Ok(None),
    }
  }

  fn serve_request(
    &self,
    req: http::Request<()>,
    stream: crate::h3_common::sessions::ServerStream,
  ) {
    let request = handle_request(
      req,
      stream,
      self.inner.router.clone(),
      self.inner.remote_addr,
    );
    let request = async move {
      if let Err(e) = request.await {
        tracing::error!("HTTP/3 request error: {e}");
      }
    };
    #[cfg(not(feature = "compio"))]
    tokio::spawn(request);
    #[cfg(feature = "compio")]
    compio::runtime::spawn(request).detach();
  }
}

async fn sleep(duration: Duration) {
  #[cfg(not(feature = "compio"))]
  tokio::time::sleep(duration).await;
  #[cfg(feature = "compio")]
  compio::time::sleep(duration).await;
}
