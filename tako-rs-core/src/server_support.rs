//! Shared lifecycle primitives for the server crates.

use std::future::Future;
use std::pin::Pin;

use futures_util::future::Either;

/// Drives a connection, notifying it once when shutdown starts.
pub async fn drive_connection<C, S, G>(connection: C, shutdown: S, graceful: G) -> C::Output
where
  C: Future,
  S: Future<Output = ()>,
  G: FnOnce(Pin<&mut C>),
{
  let mut connection = std::pin::pin!(connection);
  let shutdown = std::pin::pin!(shutdown);
  match futures_util::future::select(connection.as_mut(), shutdown).await {
    Either::Left((result, _)) => result,
    Either::Right(_) => {
      graceful(connection.as_mut());
      connection.await
    }
  }
}

/// Waits for Ctrl+C or, on Unix, SIGTERM, using a dedicated signal driver
/// when called outside Tokio.
pub async fn shutdown_signal() -> std::io::Result<()> {
  if tokio::runtime::Handle::try_current().is_ok() {
    return wait_for_os_signal().await;
  }
  let (mut sender, receiver) = tokio::sync::oneshot::channel();
  std::thread::Builder::new()
    .name("tako-signals".into())
    .spawn(move || {
      let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map(|runtime| {
          runtime.block_on(async {
            tokio::select! {
              result = wait_for_os_signal() => Some(result),
              () = sender.closed() => None,
            }
          })
        });
      match result {
        Ok(Some(result)) => {
          let _ = sender.send(result);
        }
        Err(error) => {
          let _ = sender.send(Err(error));
        }
        Ok(None) => {}
      }
    })?;
  receiver
    .await
    .map_err(|_| std::io::Error::other("signal driver stopped"))?
}

async fn wait_for_os_signal() -> std::io::Result<()> {
  #[cfg(unix)]
  {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
      result = tokio::signal::ctrl_c() => result,
      _ = terminate.recv() => Ok(()),
    }
  }
  #[cfg(not(unix))]
  tokio::signal::ctrl_c().await
}
