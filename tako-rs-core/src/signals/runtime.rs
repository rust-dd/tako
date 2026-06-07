//! Runtime-specific dispatch glue: filtered-subscription forwarding and
//! RPC timeouts, with distinct compio vs tokio spawn / sleep paths.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
#[cfg(not(feature = "compio"))]
use tokio::time::timeout;

use super::arbiter::SignalArbiter;
use super::rpc::RpcTimeoutError;
use super::signal::FILTERED_SUBSCRIPTION_BUFFER;
use super::signal::Signal;
use super::signal::SignalStream;

impl SignalArbiter {
  /// Subscribes using a filter function on top of an id-based subscription.
  ///
  /// Spawns a background task that forwards only matching signals into a
  /// bounded mpsc of capacity [`FILTERED_SUBSCRIPTION_BUFFER`]. When the
  /// consumer cannot keep up, new signals are dropped via `try_send` rather
  /// than queued unboundedly (which was the previous behavior and an OOM
  /// vector for long-running observers).
  pub fn subscribe_filtered<F>(&self, id: impl AsRef<str>, filter: F) -> SignalStream
  where
    F: Fn(&Signal) -> bool + Send + Sync + 'static,
  {
    let mut rx = self.subscribe(id);
    let (tx, out_rx) = mpsc::channel(FILTERED_SUBSCRIPTION_BUFFER);
    let filter = Arc::new(filter);

    #[cfg(not(feature = "compio"))]
    tokio::spawn(async move {
      while let Ok(signal) = rx.recv().await {
        if filter(&signal) {
          match tx.try_send(signal) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
              // Slow consumer: drop the signal silently to preserve back-pressure.
            }
            Err(mpsc::error::TrySendError::Closed(_)) => break,
          }
        }
      }
    });

    #[cfg(feature = "compio")]
    compio::runtime::spawn(async move {
      while let Ok(signal) = rx.recv().await {
        if filter(&signal) {
          match tx.try_send(signal) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {}
            Err(mpsc::error::TrySendError::Closed(_)) => break,
          }
        }
      }
    })
    .detach();

    out_rx
  }

  /// Calls a typed RPC handler with a timeout.
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

  /// Calls a typed RPC handler with a timeout (compio variant).
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
