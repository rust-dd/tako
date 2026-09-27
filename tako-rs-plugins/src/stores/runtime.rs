use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Response;

use super::StoreResult;

pub(crate) fn unavailable(error: tako_rs_core::types::BoxError) -> Response {
  tracing::error!(%error, "middleware backend failed");
  http::StatusCode::SERVICE_UNAVAILABLE.into_response()
}

pub(crate) fn spawn(future: impl Future<Output = ()> + Send + 'static) {
  #[cfg(not(feature = "compio"))]
  {
    tokio::spawn(future);
  }
  #[cfg(feature = "compio")]
  {
    compio::runtime::spawn(future).detach();
  }
}

pub(crate) async fn sleep(duration: Duration) {
  #[cfg(not(feature = "compio"))]
  tokio::time::sleep(duration).await;
  #[cfg(feature = "compio")]
  send_wrapper::SendWrapper::new(compio::time::sleep(duration)).await;
}

pub(crate) fn sweep<T, F>(store: &Arc<T>, interval: Duration, cleanup: F)
where
  T: ?Sized + Send + Sync + 'static,
  F: Fn(Arc<T>) -> BoxFuture<'static, StoreResult<()>> + Send + 'static,
{
  let weak = Arc::downgrade(store);
  spawn(async move {
    loop {
      sleep(interval).await;
      let Some(store) = weak.upgrade() else {
        break;
      };
      if let Err(error) = cleanup(store).await {
        tracing::warn!(%error, "middleware backend cleanup failed");
      }
    }
  });
}
