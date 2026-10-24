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
use crate::router::Router;
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
