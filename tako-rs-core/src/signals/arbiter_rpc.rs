//! Typed-RPC registration and call methods for the shared signal arbiter.

use std::any::Any;
use std::sync::Arc;

use super::arbiter::SignalArbiter;
use super::rpc::RpcError;
use super::rpc::RpcResult;
use super::signal::RpcHandler;

impl SignalArbiter {
  /// Registers a typed RPC handler under the given id.
  ///
  /// This allows request/response style interactions over the same arbiter,
  /// using type-erased storage internally for flexibility.
  ///
  /// # Panics
  ///
  /// The returned handler panics *at call time* (not at registration) if a
  /// caller invokes [`SignalArbiter::call_rpc`] under this id with a request
  /// type that does not match `Req`. Type erasure happens during dispatch, so
  /// the mismatch surfaces as a fail-fast panic instead of returning `None`.
  /// Keep the `Req` type stable across registrations.
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

    // `upsert_sync`: re-registering the same id replaces the prior handler.
    // `insert_sync` would keep the old one and silently drop the new closure
    // — a re-`register_rpc` after hot-reload or test reset would be a no-op.
    self.inner.rpc.upsert_sync(id_str, handler);
  }

  /// Calls a typed RPC handler and returns a shared pointer to the response.
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

  /// Calls a typed RPC handler and returns an owned response with an error type.
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

  /// Calls a typed RPC handler and returns an owned response.
  pub async fn call_rpc<Req, Res>(&self, id: impl AsRef<str>, req: Req) -> Option<Res>
  where
    Req: Send + Sync + 'static,
    Res: Send + Sync + Clone + 'static,
  {
    self.call_rpc_result::<Req, Res>(id, req).await.ok()
  }

  /// Returns a list of registered RPC ids.
  pub fn rpc_ids(&self) -> Vec<String> {
    let mut ids = Vec::new();
    self.inner.rpc.iter_sync(|k, _| {
      ids.push(k.clone());
      true
    });
    ids
  }
}
