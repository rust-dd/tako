#![cfg(feature = "plugins")]

use super::*;

#[cfg(feature = "plugins")]
#[tokio::test]
async fn compression_compresses_plain_response() {
  use tako::plugins::TakoPlugin;
  use tako::plugins::compression::CompressionBuilder;

  let mut router = Router::new();
  router.route(Method::GET, "/", |_req: Request| async move {
    let payload = "x".repeat(2048);
    http::Response::builder()
      .header("content-type", "text/plain")
      .body(TakoBody::from(payload))
      .unwrap()
  });
  CompressionBuilder::new()
    .enable_gzip(true)
    .min_size(512)
    .build()
    .setup(&router)
    .unwrap();

  let mut req = make_req(Method::GET, "/");
  req
    .headers_mut()
    .insert("accept-encoding", "gzip".parse().unwrap());
  let resp = router.dispatch(req).await;
  assert_eq!(
    resp
      .headers()
      .get("content-encoding")
      .map(|v| v.to_str().unwrap()),
    Some("gzip")
  );
}

#[cfg(feature = "plugins")]
#[tokio::test]
async fn compression_skips_when_request_authenticated() {
  use tako::plugins::TakoPlugin;
  use tako::plugins::compression::CompressionBuilder;

  let mut router = Router::new();
  router.route(Method::GET, "/", |_req: Request| async move {
    let payload = "x".repeat(2048);
    http::Response::builder()
      .header("content-type", "text/plain")
      .body(TakoBody::from(payload))
      .unwrap()
  });
  CompressionBuilder::new()
    .enable_gzip(true)
    .min_size(512)
    .build()
    .setup(&router)
    .unwrap();

  let mut req = make_req(Method::GET, "/");
  req
    .headers_mut()
    .insert("accept-encoding", "gzip".parse().unwrap());
  req
    .headers_mut()
    .insert("authorization", "Bearer secret".parse().unwrap());
  let resp = router.dispatch(req).await;
  assert!(
    !resp.headers().contains_key("content-encoding"),
    "authenticated request must not produce compressed response (CRIME mitigation)"
  );
}

#[cfg(feature = "plugins")]
#[tokio::test]
async fn compression_skips_when_response_sets_cookie() {
  use tako::plugins::TakoPlugin;
  use tako::plugins::compression::CompressionBuilder;

  let mut router = Router::new();
  router.route(Method::GET, "/", |_req: Request| async move {
    let payload = "x".repeat(2048);
    http::Response::builder()
      .header("content-type", "text/plain")
      .header("set-cookie", "sid=abc; Path=/")
      .body(TakoBody::from(payload))
      .unwrap()
  });
  CompressionBuilder::new()
    .enable_gzip(true)
    .min_size(512)
    .build()
    .setup(&router)
    .unwrap();

  let mut req = make_req(Method::GET, "/");
  req
    .headers_mut()
    .insert("accept-encoding", "gzip".parse().unwrap());
  let resp = router.dispatch(req).await;
  assert!(
    !resp.headers().contains_key("content-encoding"),
    "Set-Cookie response must not be compressed (CRIME mitigation)"
  );
}

#[cfg(feature = "plugins")]
#[tokio::test]
async fn compression_opt_out_of_crime_mitigation() {
  use tako::plugins::TakoPlugin;
  use tako::plugins::compression::CompressionBuilder;

  let mut router = Router::new();
  router.route(Method::GET, "/", |_req: Request| async move {
    let payload = "x".repeat(2048);
    http::Response::builder()
      .header("content-type", "text/plain")
      .body(TakoBody::from(payload))
      .unwrap()
  });
  CompressionBuilder::new()
    .enable_gzip(true)
    .min_size(512)
    .protect_sensitive(false)
    .build()
    .setup(&router)
    .unwrap();

  let mut req = make_req(Method::GET, "/");
  req
    .headers_mut()
    .insert("accept-encoding", "gzip".parse().unwrap());
  req
    .headers_mut()
    .insert("authorization", "Bearer secret".parse().unwrap());
  let resp = router.dispatch(req).await;
  assert_eq!(
    resp
      .headers()
      .get("content-encoding")
      .map(|v| v.to_str().unwrap()),
    Some("gzip"),
    "explicit opt-out should let auth responses be compressed"
  );
}
