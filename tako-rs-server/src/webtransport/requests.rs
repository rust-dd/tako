use std::future::Future;
use std::pin::Pin;

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use tokio::sync::mpsc;

#[cfg(not(feature = "compio"))]
pub(super) type RequestFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
#[cfg(feature = "compio")]
pub(super) type RequestFuture = Pin<Box<dyn Future<Output = ()>>>;

/// Request futures belong to the session runner, so dropping it cancels all requests.
pub(super) struct Requests {
  receiver: mpsc::UnboundedReceiver<RequestFuture>,
  pending: FuturesUnordered<RequestFuture>,
}

impl Requests {
  pub(super) fn new() -> (mpsc::UnboundedSender<RequestFuture>, Self) {
    let (sender, receiver) = mpsc::unbounded_channel();
    (
      sender,
      Self {
        receiver,
        pending: FuturesUnordered::new(),
      },
    )
  }

  pub(super) async fn drive(&mut self) {
    loop {
      tokio::select! {
        request = self.receiver.recv() => match request {
          Some(request) => self.pending.push(request),
          None => return,
        },
        Some(()) = self.pending.next(), if !self.pending.is_empty() => {}
      }
    }
  }

  pub(super) async fn drain(&mut self) {
    self.receiver.close();
    while let Ok(request) = self.receiver.try_recv() {
      self.pending.push(request);
    }
    while self.pending.next().await.is_some() {}
  }
}
