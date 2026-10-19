use bytes::Bytes;
use http::StatusCode;
use http_body_util::BodyExt;
use tako::body::TakoBody;
use tako::middleware::IntoMiddleware;
use tako::middleware::etag::ETag;
use tako::router::Router;
use tako::types::Request;

fn streamed_body() -> TakoBody {
  TakoBody::from_stream(futures_util::stream::iter([
    Ok::<_, std::io::Error>(Bytes::from_static(b"he")),
    Ok(Bytes::from_static(b"llo")),
  ]))
}

#[tokio::test]
async fn wildcard_on_unknown_length_body_can_exceed_hash_cap() {
  for max_bytes in [0, 2] {
    let mut router = Router::new();
    router.get("/", |_req: Request| async {
      http::Response::builder()
        .header("set-cookie", "first=1; Path=/")
        .header("set-cookie", "second=2; Path=/")
        .body(streamed_body())
        .unwrap()
    });
    router.middleware(ETag::new().max_bytes(max_bytes).into_middleware());

    let request = http::Request::builder()
      .header("if-none-match", "*")
      .body(TakoBody::empty())
      .unwrap();
    let response = router.dispatch(request).await;

    assert_eq!(response.status(), StatusCode::NOT_MODIFIED, "{max_bytes}");
    assert!(!response.headers().contains_key("etag"));
    assert_eq!(
      response
        .headers()
        .get_all("set-cookie")
        .iter()
        .collect::<Vec<_>>(),
      ["first=1; Path=/", "second=2; Path=/"]
    );
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

#[tokio::test]
async fn wildcard_on_small_stream_still_generates_an_etag() {
  let mut router = Router::new();
  router.get("/", |_req: Request| async {
    http::Response::new(streamed_body())
  });
  router.middleware(ETag::new().max_bytes(5).into_middleware());

  let request = http::Request::builder()
    .header("if-none-match", "*")
    .body(TakoBody::empty())
    .unwrap();
  let response = router.dispatch(request).await;

  assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
  assert_eq!(
    response.headers()["etag"],
    "W/\"2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824\""
  );
}

#[tokio::test]
async fn non_wildcard_streams_still_enforce_hash_cap() {
  for condition in [None, Some("\"different\"")] {
    let mut router = Router::new();
    router.get("/", |_req: Request| async {
      http::Response::new(streamed_body())
    });
    router.middleware(ETag::new().max_bytes(2).into_middleware());

    let mut request = http::Request::builder();
    if let Some(condition) = condition {
      request = request.header("if-none-match", condition);
    }
    let response = router
      .dispatch(request.body(TakoBody::empty()).unwrap())
      .await;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!response.headers().contains_key("etag"));
  }
}

#[tokio::test]
async fn wildcard_only_skips_stream_read_errors_with_an_existing_etag() {
  for (supplied_etag, expected) in [
    (false, StatusCode::INTERNAL_SERVER_ERROR),
    (true, StatusCode::NOT_MODIFIED),
  ] {
    let mut router = Router::new();
    router.get("/", move |_req: Request| async move {
      let body = TakoBody::from_stream(futures_util::stream::iter([Err::<Bytes, _>(
        std::io::Error::other("stream failed"),
      )]));
      let mut response = http::Response::new(body);
      if supplied_etag {
        response
          .headers_mut()
          .insert("etag", "\"hello\"".parse().unwrap());
      }
      response
    });
    router.middleware(ETag::new().max_bytes(0).into_middleware());

    let request = http::Request::builder()
      .header("if-none-match", "*")
      .body(TakoBody::empty())
      .unwrap();
    let response = router.dispatch(request).await;

    assert_eq!(response.status(), expected);
    if supplied_etag {
      assert_eq!(response.headers()["etag"], "\"hello\"");
    }
  }
}
