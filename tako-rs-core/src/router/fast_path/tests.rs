use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use http::Extensions;
use http::Method;
use http_body_util::BodyExt;

use super::Dispatch;
use crate::body::TakoBody;
use crate::conn_info::ConnInfo;
use crate::conn_info::PeerAddr;
use crate::extractors::Entries;
use crate::extractors::FromRequest;
use crate::extractors::body::BodyLimit;
use crate::extractors::params::PathParams;
use crate::recycle;
use crate::router::Router;
use crate::router_state::MatchedPath;
use crate::router_state::RouterState;
use crate::types::Request;
use crate::types::Response;

fn peer(port: u16) -> SocketAddr {
  SocketAddr::from(([127, 0, 0, 1], port))
}

fn request(method: Method, path: &str) -> Request {
  let mut req = Request::new(TakoBody::empty());
  *req.method_mut() = method;
  *req.uri_mut() = path.parse().unwrap();
  req
}

fn describe(req: &Request) -> String {
  let extensions = req.extensions();
  let conn = extensions
    .get::<ConnInfo>()
    .and_then(|info| match &info.peer {
      PeerAddr::Ip(addr) => Some(addr.port()),
      _ => None,
    })
    .filter(|port| extensions.get::<SocketAddr>().map(SocketAddr::port) == Some(*port));
  format!(
    "conn={conn:?} path={:?} params={:?} state={} limit={:?}",
    extensions.get::<MatchedPath>().map(MatchedPath::as_str),
    extensions.get::<PathParams>().map(|params| params
      .0
      .iter()
      .map(|(k, v)| format!("{k}={v}"))
      .collect::<Vec<_>>()),
    extensions.get::<Arc<RouterState>>().is_some(),
    extensions.get::<BodyLimit>().map(|limit| limit.0),
  )
}

macro_rules! sees {
  ($name:ident, $entries:expr) => {
    struct $name(String);

    impl<'a> FromRequest<'a> for $name {
      type Error = Infallible;
      const ENTRIES: Entries = $entries;

      fn from_request(
        req: &'a mut Request,
      ) -> impl Future<Output = Result<Self, Self::Error>> + Send + 'a {
        std::future::ready(Ok($name(describe(req))))
      }
    }
  };
}

sees!(SeesNothing, Entries::NONE);
sees!(SeesPath, Entries::MATCHED_PATH);
sees!(SeesConn, Entries::CONN);

/// Pools a map left behind by a request to another route on this thread.
fn pool_stale_map() {
  let mut used = Extensions::new();
  used.insert(ConnInfo::tcp(peer(1)));
  used.insert(peer(1));
  used.insert(MatchedPath("/old/{id}".into()));
  let mut params = PathParams::default();
  params.0.push(("id".into(), "stale".to_owned()));
  used.insert(params);
  used.insert(Arc::new(RouterState::new()));
  used.insert(BodyLimit(Some(1)));
  recycle::recycle_extensions(used);
}

/// Builds `req` directly on a recycled map, as a transport that attaches its
/// own entries would.
fn stale(req: &mut Request) {
  pool_stale_map();
  *req.extensions_mut() = recycle::take_extensions();
  assert_eq!(
    req.extensions().len(),
    5,
    "framework entries must survive recycling"
  );
}

async fn text(resp: Response) -> String {
  String::from_utf8(
    resp
      .into_body()
      .collect()
      .await
      .unwrap()
      .to_bytes()
      .to_vec(),
  )
  .unwrap()
}

async fn run(router: &Router, req: Request, peer: Option<SocketAddr>) -> String {
  let resp = match router.begin(req, peer) {
    Dispatch::Handler(handler) => handler.await,
    Dispatch::Full(req) => router.dispatch_full(req).await,
  };
  text(resp).await
}

fn is_handler(dispatch: &Dispatch) -> bool {
  matches!(dispatch, Dispatch::Handler(_))
}

#[test]
fn plain_routes_start_their_handler_immediately() {
  let mut router = Router::new();
  router.get("/", || async { "ok" });
  assert!(is_handler(&router.begin(request(Method::GET, "/"), None)));
}

#[test]
fn routes_with_middleware_or_timeouts_take_the_full_pipeline() {
  let mut router = Router::new();
  router
    .get("/mw", || async { "ok" })
    .middleware(|req, next| async move { next.run(req).await });
  router
    .get("/slow", || async { "ok" })
    .timeout(Duration::from_secs(1));
  assert!(!is_handler(
    &router.begin(request(Method::GET, "/mw"), None)
  ));
  assert!(!is_handler(
    &router.begin(request(Method::GET, "/slow"), None)
  ));

  let mut timed = Router::new();
  timed.get("/", || async { "ok" });
  timed.timeout(Duration::from_secs(1));
  assert!(!is_handler(&timed.begin(request(Method::GET, "/"), None)));
}

#[test]
fn head_unmatched_and_error_hooked_requests_take_the_full_pipeline() {
  let mut router = Router::new();
  router.get("/", || async { "ok" });
  assert!(!is_handler(&router.begin(request(Method::HEAD, "/"), None)));
  assert!(!is_handler(
    &router.begin(request(Method::GET, "/missing"), None)
  ));

  let mut hooked = Router::new();
  hooked.get("/", || async { "ok" });
  hooked.error_handler(|resp| resp);
  assert!(!is_handler(&hooked.begin(request(Method::GET, "/"), None)));
}

#[tokio::test]
async fn plain_routes_get_only_the_entries_their_extractors_list() {
  let mut router = Router::new();
  router.get(
    "/none",
    |SeesNothing(seen): SeesNothing| async move { seen },
  );
  router.get("/path", |SeesPath(seen): SeesPath| async move { seen });
  router.get("/conn", |SeesConn(seen): SeesConn| async move { seen });
  for (path, expected) in [
    (
      "/none",
      "conn=None path=None params=None state=false limit=None",
    ),
    (
      "/path",
      "conn=None path=Some(\"/path\") params=None state=false limit=None",
    ),
    (
      "/conn",
      "conn=Some(4000) path=None params=None state=false limit=None",
    ),
  ] {
    pool_stale_map();
    let seen = run(&router, request(Method::GET, path), Some(peer(4000))).await;
    assert_eq!(seen, expected, "{path}");
  }
}

#[tokio::test]
async fn the_full_pipeline_gets_every_entry() {
  let mut router = Router::new();
  router
    .get("/", |req: Request| async move { describe(&req) })
    .middleware(|req, next| async move { next.run(req).await });
  pool_stale_map();
  assert_eq!(
    run(&router, request(Method::GET, "/"), Some(peer(4000))).await,
    "conn=Some(4000) path=Some(\"/\") params=None state=false limit=None"
  );
}

#[tokio::test]
async fn recycled_entries_are_rewritten_for_the_matched_route() {
  let mut router = Router::new();
  router.get("/", |req: Request| async move { describe(&req) });
  let mut req = request(Method::GET, "/");
  stale(&mut req);
  assert_eq!(
    run(&router, req, None).await,
    "conn=Some(1) path=Some(\"/\") params=None state=false limit=None"
  );
}

#[tokio::test]
async fn path_parameters_reuse_recycled_entries() {
  let mut router = Router::new();
  router.get("/users/{id}", |req: Request| async move { describe(&req) });
  router.body_limit(64);
  let mut req = request(Method::GET, "/users/42");
  stale(&mut req);
  assert_eq!(
    text(router.dispatch(req).await).await,
    "conn=Some(1) path=Some(\"/users/{id}\") params=Some([\"id=42\"]) state=false limit=Some(Some(64))"
  );
}

#[tokio::test]
async fn unmatched_requests_drop_recycled_route_entries() {
  let mut router = Router::new();
  router.get("/", || async { "ok" });
  router.fallback(|req: Request| async move { describe(&req) });
  let mut req = request(Method::GET, "/missing");
  stale(&mut req);
  assert_eq!(
    text(router.dispatch(req).await).await,
    "conn=Some(1) path=None params=None state=false limit=None"
  );
}

#[test]
fn connection_entries_overwrite_a_recycled_map() {
  pool_stale_map();
  let mut extensions = Extensions::new();
  super::attach_conn(&mut extensions, peer(2));
  assert_eq!(extensions.get::<SocketAddr>(), Some(&peer(2)));
  assert!(matches!(
    extensions.get::<ConnInfo>().map(|info| &info.peer),
    Some(PeerAddr::Ip(addr)) if *addr == peer(2)
  ));
}

#[test]
fn entries_a_transport_already_added_are_kept() {
  let mut extensions = Extensions::new();
  extensions.insert(7_u8);
  super::attach_conn(&mut extensions, peer(2));
  assert_eq!(extensions.get::<u8>(), Some(&7));
  assert_eq!(extensions.get::<SocketAddr>(), Some(&peer(2)));
}
