#![cfg(feature = "http3")]

#[path = "support/h3.rs"]
mod support;

use std::convert::Infallible;
use std::net::SocketAddr;

use bytes::Buf;
use tako::extractors::connect_info::ConnectInfo;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::conn_info::Transport;
use tako_rs_core::extractors::Entries;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;

struct NoConnection;

impl<'a> FromRequest<'a> for NoConnection {
  type Error = Infallible;
  const ENTRIES: Entries = Entries::NONE;

  fn from_request(
    req: &'a mut Request,
  ) -> impl std::future::Future<Output = Result<Self, Self::Error>> + Send + 'a {
    assert!(req.extensions().get::<ConnInfo>().is_none());
    assert!(req.extensions().get::<SocketAddr>().is_none());
    std::future::ready(Ok(Self))
  }
}

fn assert_metadata(info: &ConnInfo) {
  assert_eq!(info.transport, Transport::Http3);
  assert!(info.peer.as_socket().unwrap().ip().is_loopback());
  let tls = info.tls.as_ref().unwrap();
  assert_eq!(tls.alpn.as_deref(), Some(b"h3".as_slice()));
  assert_eq!(tls.version, Some("TLSv1.3"));
}

fn assert_request(req: &Request) {
  assert_eq!(req.version(), http::Version::HTTP_3);
  let info = req.extensions().get::<ConnInfo>().unwrap();
  assert_metadata(info);
  assert_eq!(req.extensions().get::<SocketAddr>(), info.peer.as_socket());
}

#[test]
fn h3_preserves_metadata_for_extractors_middleware_and_fallback() {
  let mut router = Router::new();
  router.get("/plain", |_: NoConnection| async { "ok" });
  router.get(
    "/typed",
    |ConnectInfo(info): ConnectInfo<ConnInfo>| async move {
      assert_metadata(&info);
      "ok"
    },
  );
  router.get("/raw", |req: Request| async move {
    assert_request(&req);
    "ok"
  });
  router
    .get("/middleware", || async { "ok" })
    .middleware(|req, next| async move {
      assert_request(&req);
      next.run(req).await
    });
  router.fallback(|req: Request| async move {
    assert_request(&req);
    "ok"
  });
  support::run(router, |mut client| async move {
    for path in [
      "/plain",
      "/typed",
      "/raw",
      "/middleware",
      "/missing",
      "/plain",
    ] {
      let mut stream = client
        .sender
        .send_request(
          http::Request::builder()
            .uri(format!("https://localhost{path}"))
            .body(())
            .unwrap(),
        )
        .await
        .unwrap();
      if let Err(error) = stream.finish().await {
        // An early response may cancel the upload before FIN with H3_NO_ERROR.
        assert!(
          matches!(
            &error,
            h3::error::StreamError::RemoteTerminate {
              code: h3::error::Code::H3_NO_ERROR,
              ..
            }
          ),
          "{error}"
        );
      }
      assert_eq!(
        stream.recv_response().await.unwrap().status(),
        http::StatusCode::OK
      );
      let mut body = Vec::new();
      while let Some(mut chunk) = stream.recv_data().await.unwrap() {
        body.extend_from_slice(&chunk.copy_to_bytes(chunk.remaining()));
      }
      assert_eq!(body, b"ok", "{path}");
    }
    client.close().await;
  });
}
