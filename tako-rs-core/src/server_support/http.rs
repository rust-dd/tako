use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;

use futures_util::future::Either;
use hyper::body::Incoming;
use hyper::service::Service;

use super::connection_router;
use crate::body::TakoBody;
use crate::conn_info::ConnInfo;
use crate::router::Dispatch;
use crate::router::Router;
use crate::types::Request;
use crate::types::Response;

/// Hyper service with connection metadata and a compact response future for multiplexed
/// streams. Plain handlers start on the connection task; only the full pipeline is boxed.
pub fn http_service(
  router: Arc<Router>,
  info: ConnInfo,
) -> impl Service<
  hyper::Request<Incoming>,
  Response = Response,
  Error = Infallible,
  Future = impl Future<Output = Result<Response, Infallible>>,
> {
  let router = connection_router(&router);
  hyper::service::service_fn(move |req: hyper::Request<Incoming>| {
    start_request(Arc::clone(&router), req.map(TakoBody::incoming), &info)
  })
}

#[allow(
  clippy::redundant_allocation,
  reason = "Each connection needs its own reference count instead of sharing the endpoint counter"
)]
fn start_request(
  router: Arc<Arc<Router>>,
  mut req: Request,
  info: &ConnInfo,
) -> impl Future<Output = Result<Response, Infallible>> + use<> {
  if let Some(peer) = info.peer.as_socket() {
    req.extensions_mut().insert(*peer);
  }
  req.extensions_mut().insert(info.clone());
  let pending = match router.begin(req, None) {
    Dispatch::Handler(handler) => Either::Left(handler),
    Dispatch::Full(req) => Either::Right(Box::pin(async move { router.dispatch_full(req).await })),
  };
  async move { Ok(pending.await) }
}

#[cfg(test)]
mod tests;
