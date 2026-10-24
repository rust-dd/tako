use std::time::Duration;

use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
#[cfg(not(feature = "compio"))]
use tokio::time::timeout;

use super::arbiter::SignalArbiter;
use super::rpc::RpcTimeoutError;
use super::signal::FILTERED_SUBSCRIPTION_BUFFER;
use super::signal::Signal;
use super::signal::SignalStream;

impl SignalArbiter {
  /// Subscribes to `id`, forwarding the signals `filter` accepts into a [`SignalStream`] that
  /// drops them while it is full.
  pub fn subscribe_filtered<F>(&self, id: impl AsRef<str>, filter: F) -> SignalStream
  where
    F: Fn(&Signal) -> bool + Send + Sync + 'static,
  {
    let mut rx = self.subscribe(id);
    let (tx, out_rx) = mpsc::channel(FILTERED_SUBSCRIPTION_BUFFER);
    let forward = async move {
      loop {
        let received = tokio::select! {
          biased;
          () = tx.closed() => break,
          received = rx.recv() => received,
        };
        let signal = match received {
          Ok(signal) => signal,
          Err(RecvError::Lagged(_)) => continue,
          Err(RecvError::Closed) => break,
        };
        if filter(&signal)
          && let Err(mpsc::error::TrySendError::Closed(_)) = tx.try_send(signal)
        {
          break;
        }
      }
    };

    #[cfg(not(feature = "compio"))]
    tokio::spawn(forward);
    #[cfg(feature = "compio")]
    compio::runtime::spawn(forward).detach();

    out_rx
  }

  #[cfg(not(feature = "compio"))]
  pub async fn call_rpc_timeout<Req, Res>(
    &self,
    id: impl AsRef<str>,
    req: Req,
    dur: Duration,
  ) -> Result<Res, RpcTimeoutError>
  where
    Req: Send + Sync + 'static,
    Res: Send + Sync + Clone + 'static,
  {
    match timeout(dur, self.call_rpc_result::<Req, Res>(id, req)).await {
      Ok(Ok(res)) => Ok(res),
      Ok(Err(e)) => Err(RpcTimeoutError::Rpc(e)),
      Err(_) => Err(RpcTimeoutError::Timeout),
    }
  }

  #[cfg(feature = "compio")]
  pub async fn call_rpc_timeout<Req, Res>(
    &self,
    id: impl AsRef<str>,
    req: Req,
    dur: Duration,
  ) -> Result<Res, RpcTimeoutError>
  where
    Req: Send + Sync + 'static,
    Res: Send + Sync + Clone + 'static,
  {
    let sleep = std::pin::pin!(compio::time::sleep(dur));
    let work = std::pin::pin!(self.call_rpc_result::<Req, Res>(id, req));
    match futures_util::future::select(work, sleep).await {
      futures_util::future::Either::Left((Ok(res), _)) => Ok(res),
      futures_util::future::Either::Left((Err(e), _)) => Err(RpcTimeoutError::Rpc(e)),
      futures_util::future::Either::Right(((), _)) => Err(RpcTimeoutError::Timeout),
    }
  }
}
