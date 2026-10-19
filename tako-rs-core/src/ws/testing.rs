//! Client-side frame builders for tests.

pub(crate) const MASK: [u8; 4] = [0x37, 0xfa, 0x21, 0x3d];
pub(crate) const TEXT: u8 = 0x81;
pub(crate) const BINARY: u8 = 0x82;
pub(crate) const CLOSE: u8 = 0x88;
pub(crate) const PING: u8 = 0x89;
pub(crate) const PONG: u8 = 0x8A;
pub(crate) const TEXT_START: u8 = 0x01;
pub(crate) const CONTINUE: u8 = 0x00;
pub(crate) const CONTINUE_END: u8 = 0x80;

/// A masked client frame whose first byte (FIN and opcode) is `first`.
pub(crate) fn client_frame(first: u8, payload: &[u8]) -> Vec<u8> {
  let mut frame = vec![first];
  match payload.len() {
    len @ 0..=125 => frame.push(0x80 | len as u8),
    len @ 126..=0xFFFF => {
      frame.push(0xFE);
      frame.extend_from_slice(&(len as u16).to_be_bytes());
    }
    len => {
      frame.push(0xFF);
      frame.extend_from_slice(&(len as u64).to_be_bytes());
    }
  }
  frame.extend_from_slice(&MASK);
  frame.extend(
    payload
      .iter()
      .zip(MASK.iter().cycle())
      .map(|(byte, key)| byte ^ key),
  );
  frame
}

/// The payload of a close frame with `code` and `reason`.
pub(crate) fn close_payload(code: u16, reason: &str) -> Vec<u8> {
  let mut payload = code.to_be_bytes().to_vec();
  payload.extend_from_slice(reason.as_bytes());
  payload
}

pub(crate) use pipe::Counts;
pub(crate) use pipe::upgraded_pair;

mod pipe {
  use std::convert::Infallible;
  use std::io;
  use std::pin::Pin;
  use std::sync::Arc;
  use std::sync::atomic::AtomicUsize;
  use std::sync::atomic::Ordering;
  use std::task::Context;
  use std::task::Poll;

  use bytes::Bytes;
  use http_body_util::Empty;
  use hyper::upgrade::Upgraded;
  use hyper_util::rt::TokioIo;
  use tokio::io::AsyncRead;
  use tokio::io::AsyncReadExt;
  use tokio::io::AsyncWrite;
  use tokio::io::AsyncWriteExt;
  use tokio::io::DuplexStream;
  use tokio::io::ReadBuf;

  /// Successful reads and writes on the server end of a test pipe.
  #[derive(Clone, Default)]
  pub(crate) struct Counts {
    reads: Arc<AtomicUsize>,
    writes: Arc<AtomicUsize>,
  }

  impl Counts {
    pub(crate) fn reads(&self) -> usize {
      self.reads.load(Ordering::SeqCst)
    }

    pub(crate) fn writes(&self) -> usize {
      self.writes.load(Ordering::SeqCst)
    }
  }

  struct Counting {
    inner: DuplexStream,
    counts: Counts,
  }

  impl AsyncRead for Counting {
    fn poll_read(
      mut self: Pin<&mut Self>,
      cx: &mut Context<'_>,
      buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
      let before = buf.filled().len();
      let result = Pin::new(&mut self.inner).poll_read(cx, buf);
      if matches!(result, Poll::Ready(Ok(()))) && buf.filled().len() > before {
        self.counts.reads.fetch_add(1, Ordering::SeqCst);
      }
      result
    }
  }

  impl AsyncWrite for Counting {
    fn poll_write(
      mut self: Pin<&mut Self>,
      cx: &mut Context<'_>,
      buf: &[u8],
    ) -> Poll<io::Result<usize>> {
      let result = Pin::new(&mut self.inner).poll_write(cx, buf);
      if matches!(result, Poll::Ready(Ok(written)) if written > 0) {
        self.counts.writes.fetch_add(1, Ordering::SeqCst);
      }
      result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
      Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
      Pin::new(&mut self.inner).poll_shutdown(cx)
    }
  }

  /// The server end of an upgraded connection over a pipe of `capacity` bytes, the client end,
  /// and counters for the server end that start at zero after the upgrade.
  pub(crate) async fn upgraded_pair(capacity: usize) -> (Upgraded, DuplexStream, Counts) {
    let (mut client, server) = tokio::io::duplex(capacity);
    let counts = Counts::default();
    let io = Counting {
      inner: server,
      counts: counts.clone(),
    };
    let (upgrades, mut upgrade) = tokio::sync::mpsc::unbounded_channel();
    let service = hyper::service::service_fn(move |mut request| {
      let _ = upgrades.send(hyper::upgrade::on(&mut request));
      async {
        Ok::<_, Infallible>(
          hyper::Response::builder()
            .status(hyper::StatusCode::SWITCHING_PROTOCOLS)
            .header(hyper::header::UPGRADE, "websocket")
            .header(hyper::header::CONNECTION, "upgrade")
            .body(Empty::<Bytes>::new())
            .unwrap(),
        )
      }
    });
    crate::ws::runtime::spawn(async move {
      let _ = hyper::server::conn::http1::Builder::new()
        .serve_connection(TokioIo::new(io), service)
        .with_upgrades()
        .await;
    });
    client
      .write_all(
        b"GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: upgrade\r\n\r\n",
      )
      .await
      .unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
      head.push(client.read_u8().await.unwrap());
    }
    let upgraded = upgrade.recv().await.unwrap().await.unwrap();
    counts.reads.store(0, Ordering::SeqCst);
    counts.writes.store(0, Ordering::SeqCst);
    (upgraded, client, counts)
  }
}

#[cfg(feature = "compio")]
pub(crate) use compio::runtime::spawn;
#[cfg(not(feature = "compio"))]
pub(crate) use tokio::spawn;

#[cfg(not(feature = "compio"))]
pub(crate) trait TaskFuture: std::future::Future + Send + 'static {}
#[cfg(not(feature = "compio"))]
impl<F: std::future::Future + Send + 'static> TaskFuture for F {}
#[cfg(feature = "compio")]
pub(crate) trait TaskFuture: std::future::Future + 'static {}
#[cfg(feature = "compio")]
impl<F: std::future::Future + 'static> TaskFuture for F {}

pub(crate) async fn timeout<F: std::future::Future>(
  duration: std::time::Duration,
  future: F,
) -> Result<F::Output, ()> {
  let timer = crate::ws::runtime::sleep(duration);
  futures_util::pin_mut!(future);
  match futures_util::future::select(future, timer).await {
    futures_util::future::Either::Left((output, _)) => Ok(output),
    futures_util::future::Either::Right(_) => Err(()),
  }
}

pub(crate) async fn yield_now() {
  let mut yielded = false;
  std::future::poll_fn(|cx| {
    if yielded {
      std::task::Poll::Ready(())
    } else {
      yielded = true;
      cx.waker().wake_by_ref();
      std::task::Poll::Pending
    }
  })
  .await;
}

#[cfg(not(feature = "compio"))]
pub(crate) async fn cancel<T>(task: tokio::task::JoinHandle<T>) {
  task.abort();
  let _ = task.await;
}

#[cfg(feature = "compio")]
pub(crate) async fn cancel<T>(task: compio::runtime::JoinHandle<T>) {
  let _ = task.cancel().await;
}
