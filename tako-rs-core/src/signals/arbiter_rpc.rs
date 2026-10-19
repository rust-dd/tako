use std::any::Any;
use std::sync::Arc;

use super::arbiter::SignalArbiter;
use super::rpc::RpcError;
use super::rpc::RpcResult;
use super::signal::RpcHandler;

impl SignalArbiter {
  /// Registers the RPC handler for `id`, replacing any previous one; a call with a request
  /// type other than `Req` panics.
  pub fn register_rpc<Req, Res, F, Fut>(&self, id: impl Into<String>, f: F)
  where
    Req: Send + Sync + 'static,
    Res: Send + Sync + 'static,
    F: Fn(Arc<Req>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Res> + Send + 'static,
  {
    let id_str = id.into();
    let id_for_panic = id_str.clone();
    let func = Arc::new(f);

    let handler: RpcHandler = Arc::new(move |raw: Arc<dyn Any + Send + Sync>| {
      let func = func.clone();
      let id_for_panic = id_for_panic.clone();
      Box::pin(async move {
        let req = raw
          .downcast::<Req>()
          .unwrap_or_else(|_| panic!("Signal RPC type mismatch for id: {id_for_panic}"));
        let res = func(req).await;
        Arc::new(res) as Arc<dyn Any + Send + Sync>
      })
    });

    self.inner.rpc.upsert_sync(id_str, handler);
  }

  /// Calls the RPC for `id`; `None` without a handler or when the response is not `Res`.
  pub async fn call_rpc_arc<Req, Res>(&self, id: impl AsRef<str>, req: Req) -> Option<Arc<Res>>
  where
    Req: Send + Sync + 'static,
    Res: Send + Sync + 'static,
  {
    let id_str = id.as_ref();
    let entry = self.inner.rpc.get_async(id_str).await?;
    let handler = entry.clone();
    drop(entry);

    let raw_req: Arc<dyn Any + Send + Sync> = Arc::new(req);
    let raw_res = handler(raw_req).await;

    raw_res.downcast::<Res>().ok()
  }

  /// Like [`call_rpc_arc`](Self::call_rpc_arc), but clones the response and says why it
  /// failed.
  pub async fn call_rpc_result<Req, Res>(&self, id: impl AsRef<str>, req: Req) -> RpcResult<Res>
  where
    Req: Send + Sync + 'static,
    Res: Send + Sync + Clone + 'static,
  {
    let id_str = id.as_ref();
    let Some(entry) = self.inner.rpc.get_async(id_str).await else {
      return Err(RpcError::NoHandler);
    };
    let handler = entry.clone();
    drop(entry);

    let raw_req: Arc<dyn Any + Send + Sync> = Arc::new(req);
    let raw_res = handler(raw_req).await;

    match raw_res.downcast::<Res>() {
      Ok(res) => Ok((*res).clone()),
      Err(_) => Err(RpcError::TypeMismatch),
    }
  }

  /// [`call_rpc_result`](Self::call_rpc_result) without the reason.
  pub async fn call_rpc<Req, Res>(&self, id: impl AsRef<str>, req: Req) -> Option<Res>
  where
    Req: Send + Sync + 'static,
    Res: Send + Sync + Clone + 'static,
  {
    self.call_rpc_result::<Req, Res>(id, req).await.ok()
  }

  pub fn rpc_ids(&self) -> Vec<String> {
    let mut ids = Vec::new();
    self.inner.rpc.iter_sync(|k, _| {
      ids.push(k.clone());
      true
    });
    ids
  }
}
