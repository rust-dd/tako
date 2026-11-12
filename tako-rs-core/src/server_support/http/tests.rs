use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http::Method;
use http::StatusCode;
use http::Version;
use http_body_util::BodyExt;

use super::start_request;
use crate::body::TakoBody;
use crate::conn_info::ConnInfo;
use crate::conn_info::TlsInfo;
use crate::conn_info::Transport;
use crate::extractors::Entries;
use crate::extractors::FromRequest;
use crate::router::Router;
use crate::router_state::MatchedPath;
use crate::server_support::connection_router;
use crate::types::Request;

fn request(method: Method, path: &str) -> Request {
  http::Request::builder()
    .method(method)
    .uri(path)
    .version(Version::HTTP_2)
    .body(TakoBody::empty())
    .unwrap()
}

fn peer() -> SocketAddr {
  SocketAddr::from(([127, 0, 0, 1], 12345))
}

#[tokio::test]
async fn plain_handler_keeps_http2_peer_and_tls_metadata_across_yield() {
  let mut router = Router::new();
  router.get("/", |req: Request| async move {
    tokio::task::yield_now().await;
    assert_eq!(req.version(), Version::HTTP_2);
    assert_eq!(req.extensions().get::<SocketAddr>(), Some(&peer()));
    let info = req.extensions().get::<ConnInfo>().unwrap();
    assert_eq!(info.transport, Transport::Http2);
    assert_eq!(info.peer.as_socket(), Some(&peer()));
    let tls = info.tls.as_ref().unwrap();
    assert_eq!(tls.alpn.as_deref(), Some(b"h2".as_slice()));
    assert_eq!(tls.sni.as_deref(), Some("example.com"));
    assert_eq!(tls.version, Some("TLSv1.3"));
    "ok"
  });
  let router = Arc::new(router);
  let info = ConnInfo::h2_tls(
    peer(),
    TlsInfo {
      alpn: Some(Bytes::from_static(b"h2")),
      sni: Some(Arc::from("example.com")),
      version: Some("TLSv1.3"),
    },
  );
  let response = start_request(connection_router(&router), request(Method::GET, "/"), &info)
    .await
    .unwrap();
  assert_eq!(response.status(), StatusCode::OK);
  assert_eq!(
    response.into_body().collect().await.unwrap().to_bytes(),
    "ok"
  );
}

#[tokio::test]
async fn full_pipeline_keeps_metadata_for_head_and_fallback() {
  let mut router = Router::new();
  router.get("/", || async { "hello" });
  router.middleware(|req, next| async move {
    let info = req.extensions().get::<ConnInfo>().unwrap();
    assert_eq!(info.transport, Transport::Http2);
    assert_eq!(info.peer.as_socket(), Some(&peer()));
    assert!(info.tls.is_none());
    let mut response = next.run(req).await;
    response
      .headers_mut()
      .insert("x-middleware", "ran".parse().unwrap());
    response
  });
  let router = Arc::new(router);
  let connection = connection_router(&router);
  let info = ConnInfo::h2c(peer());
  for (method, path, status) in [
    (Method::HEAD, "/", StatusCode::OK),
    (Method::GET, "/missing", StatusCode::NOT_FOUND),
  ] {
    let response = start_request(Arc::clone(&connection), request(method, path), &info)
      .await
      .unwrap();
    assert_eq!(response.status(), status);
    assert_eq!(response.headers()["x-middleware"], "ran");
    if status == StatusCode::OK {
      assert_eq!(response.headers()[http::header::CONTENT_LENGTH], "5");
    }
    assert!(
      response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .is_empty()
    );
  }
}

#[test]
fn pending_streams_do_not_retain_endpoint_router_references() {
  let mut router = Router::new();
  router.middleware(|req, next| async move { next.run(req).await });
  router.get("/", || async { "ok" });
  let router = Arc::new(router);
  let connection = connection_router(&router);
  let info = ConnInfo::h2c(peer());
  let pending: Vec<_> = (0..100)
    .map(|_| start_request(Arc::clone(&connection), request(Method::GET, "/"), &info))
    .collect();
  assert_eq!(Arc::strong_count(&router), 2);
  drop(pending);
  drop(connection);
  assert_eq!(Arc::strong_count(&router), 1);
}

struct WithoutConnection<const MATCHED_PATH: bool>(bool);

impl<'a, const MATCHED_PATH: bool> FromRequest<'a> for WithoutConnection<MATCHED_PATH> {
  type Error = std::convert::Infallible;
  const ENTRIES: Entries = if MATCHED_PATH {
    Entries::MATCHED_PATH
  } else {
    Entries::NONE
  };

  fn from_request(
    req: &'a mut Request,
  ) -> impl std::future::Future<Output = Result<Self, Self::Error>> + Send + 'a {
    assert!(req.extensions().get::<ConnInfo>().is_none());
    assert!(req.extensions().get::<SocketAddr>().is_none());
    assert!(
      req
        .extensions()
        .get::<crate::extractors::body::BodyLimit>()
        .is_none()
    );
    assert_eq!(
      req
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str),
      MATCHED_PATH.then_some("/listed")
    );
    std::future::ready(Ok(Self(req.extensions().get::<u32>() == Some(&42))))
  }
}

#[tokio::test]
async fn plain_extractors_skip_unrequested_connection_metadata() {
  let mut router = Router::new();
  router.get(
    "/none",
    |WithoutConnection(custom): WithoutConnection<false>| async move {
      if custom { "custom" } else { "ok" }
    },
  );
  router.get(
    "/listed",
    |WithoutConnection(custom): WithoutConnection<true>| async move {
      if custom { "custom" } else { "ok" }
    },
  );
  let router = Arc::new(router);
  for (version, info) in [
    (
      Version::HTTP_11,
      ConnInfo::h1_tls(peer(), TlsInfo::default()),
    ),
    (Version::HTTP_2, ConnInfo::h2c(peer())),
    (
      Version::HTTP_2,
      ConnInfo::h2_tls(peer(), TlsInfo::default()),
    ),
    (Version::HTTP_3, ConnInfo::h3(peer(), TlsInfo::default())),
  ] {
    for path in ["/none", "/listed"] {
      let mut req = request(Method::GET, path);
      *req.version_mut() = version;
      req.extensions_mut().insert(42_u32);
      let response = start_request(connection_router(&router), req, &info)
        .await
        .unwrap();
      assert_eq!(response.status(), StatusCode::OK);
      assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "custom"
      );
      let mut stale = http::Extensions::new();
      stale.insert(ConnInfo::tcp(SocketAddr::from(([127, 0, 0, 2], 54321))));
      stale.insert(SocketAddr::from(([127, 0, 0, 2], 54321)));
      stale.insert(MatchedPath("/stale".into()));
      stale.insert(crate::extractors::body::BodyLimit(Some(1)));
      crate::recycle::recycle_extensions(stale);
      let mut req = request(Method::GET, path);
      *req.version_mut() = version;
      let response = start_request(connection_router(&router), req, &info)
        .await
        .unwrap();
      assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "ok"
      );
    }
  }
}

struct ConnectionMetadata(ConnInfo);

impl<'a> FromRequest<'a> for ConnectionMetadata {
  type Error = std::convert::Infallible;
  const ENTRIES: Entries = Entries::CONN;

  fn from_request(
    req: &'a mut Request,
  ) -> impl std::future::Future<Output = Result<Self, Self::Error>> + Send + 'a {
    let info = req.extensions().get::<ConnInfo>().unwrap();
    assert_eq!(req.extensions().get::<SocketAddr>(), info.peer.as_socket());
    std::future::ready(Ok(Self(info.clone())))
  }
}

#[tokio::test]
async fn listed_connection_metadata_is_replaced_across_transports() {
  let mut router = Router::new();
  router.get(
    "/",
    |ConnectionMetadata(info): ConnectionMetadata| async move {
      let tls = info.tls.unwrap_or_default();
      format!(
        "{:?} {:?} {} {}",
        info.transport,
        info.peer.as_socket(),
        tls.sni.as_deref().unwrap_or("none"),
        std::str::from_utf8(tls.alpn.as_deref().unwrap_or(b"none")).unwrap()
      )
    },
  );
  let router = Arc::new(router);
  let tls = TlsInfo {
    alpn: Some(Bytes::from_static(b"h2")),
    sni: Some(Arc::from("example.com")),
    version: Some("TLSv1.3"),
  };
  for (info, expected) in [
    (
      ConnInfo::h2_tls(peer(), tls),
      "Http2 Some(127.0.0.1:12345) example.com h2",
    ),
    (
      ConnInfo::h2c(peer()),
      "Http2 Some(127.0.0.1:12345) none none",
    ),
    (
      ConnInfo::h3(
        peer(),
        TlsInfo {
          alpn: Some(Bytes::from_static(b"h3")),
          ..TlsInfo::default()
        },
      ),
      "Http3 Some(127.0.0.1:12345) none h3",
    ),
    (ConnInfo::unix(None), "Unix None none none"),
    (
      ConnInfo::tcp(peer()),
      "Http1 Some(127.0.0.1:12345) none none",
    ),
  ] {
    let response = super::dispatch_with_info(&router, request(Method::GET, "/"), &info).await;
    assert_eq!(
      response.into_body().collect().await.unwrap().to_bytes(),
      expected
    );
  }
}

#[tokio::test]
async fn error_formatter_never_observes_a_previous_requests_route_entries() {
  use crate::extractors::body::BodyLimit;
  use crate::extractors::params::PathParams;

  let mut previous = Router::new();
  previous.get("/users/{id}", || async { "ok" });
  previous.body_limit(64);
  previous.middleware(|req, next| async move { next.run(req).await });
  let mut router = Router::new();
  router.get("/error", || async { StatusCode::BAD_REQUEST });
  router.error_handler_with_parts(|parts, response| {
    assert_eq!(parts.uri.path(), "/error");
    assert!(parts.extensions.get::<PathParams>().is_none());
    assert!(parts.extensions.get::<BodyLimit>().is_none());
    assert_eq!(
      parts.extensions.get::<MatchedPath>().unwrap().as_str(),
      "/error"
    );
    assert_eq!(parts.extensions.get::<SocketAddr>(), Some(&peer()));
    response
  });
  let info = ConnInfo::h2c(peer());
  let response =
    super::dispatch_with_info(&previous, request(Method::GET, "/users/secret"), &info).await;
  assert_eq!(response.status(), StatusCode::OK);
  let response = super::dispatch_with_info(&router, request(Method::GET, "/error"), &info).await;
  assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[test]
#[cfg(not(feature = "compio"))]
fn full_request_body_limits_survive_moving_to_another_worker() {
  use crate::extractors::body::BodyLimit;

  let mut router = Router::new();
  router.post("/", |_: Bytes| async { StatusCode::OK });
  router.middleware(|req, next| async move { next.run(req).await });
  let router = Arc::new(router);
  for (stale_limit, size, expected) in [
    (None, 2 * 1024 * 1024 + 1, StatusCode::PAYLOAD_TOO_LARGE),
    (Some(1), 2, StatusCode::OK),
  ] {
    let mut stale = http::Extensions::new();
    stale.insert(BodyLimit(stale_limit));
    crate::recycle::recycle_extensions(stale);
    let mut req = request(Method::POST, "/");
    *req.body_mut() = TakoBody::from(vec![0; size]);
    let pending = start_request(connection_router(&router), req, &ConnInfo::h2c(peer()));
    let status = std::thread::spawn(move || {
      tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(pending)
        .unwrap()
        .status()
    })
    .join()
    .unwrap();
    assert_eq!(status, expected, "stale_limit={stale_limit:?}");
  }
}
