use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;

use futures_util::future::Either;
use hyper::body::Incoming;
use hyper::service::Service;

use super::connection_router;
use super::sanitize_response;
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
  service(router, info, None)
}

/// HTTP service with supervised WebSocket sessions.
pub fn http_service_with_upgrades(
  router: Arc<Router>,
  info: ConnInfo,
  upgrades: super::UpgradeRegistry,
) -> impl Service<
  hyper::Request<Incoming>,
  Response = Response,
  Error = Infallible,
  Future = impl Future<Output = Result<Response, Infallible>>,
> {
  service(router, info, Some(upgrades))
}

fn service(
  router: Arc<Router>,
  info: ConnInfo,
  upgrades: Option<super::UpgradeRegistry>,
) -> impl Service<
  hyper::Request<Incoming>,
  Response = Response,
  Error = Infallible,
  Future = impl Future<Output = Result<Response, Infallible>>,
> {
  let router = connection_router(&router);
  hyper::service::service_fn(move |mut req: hyper::Request<Incoming>| {
    if let Some(upgrades) = &upgrades {
      upgrades.attach(&mut req);
    }
    start_request(Arc::clone(&router), req.map(TakoBody::incoming), &info)
  })
}

#[allow(
  clippy::redundant_allocation,
  reason = "Each connection needs its own reference count instead of sharing the endpoint counter"
)]
fn start_request(
  router: Arc<Arc<Router>>,
  req: Request,
  info: &ConnInfo,
) -> impl Future<Output = Result<Response, Infallible>> + use<> {
  let multiplexed = matches!(req.version(), http::Version::HTTP_2 | http::Version::HTTP_3);
  let pending = match router.begin_with_info(req, Some(info)) {
    Dispatch::Handler(handler) => Either::Left(handler),
    Dispatch::Full(req) => Either::Right(Box::pin(async move { router.dispatch_full(req).await })),
  };
  async move {
    let response = pending.await;
    Ok(if multiplexed {
      sanitize_response(response)
    } else {
      response
    })
  }
}

/// Dispatches a transport request, providing connection metadata to every middleware and
/// to handlers whose extractors need it. Pre-existing request extensions are preserved.
pub async fn dispatch_with_info(router: &Router, req: Request, info: &ConnInfo) -> Response {
  match router.begin_with_info(req, Some(info)) {
    Dispatch::Handler(handler) => handler.await,
    Dispatch::Full(req) => router.dispatch_full(req).await,
  }
}

#[cfg(test)]
mod tests;
