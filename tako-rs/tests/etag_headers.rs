use http::Method;
use http::StatusCode;
use http_body_util::BodyExt;
use tako::body::TakoBody;
use tako::middleware::IntoMiddleware;
use tako::middleware::etag::ETag;
use tako::router::Router;
use tako::types::Request;

#[tokio::test]
async fn not_modified_preserves_every_header_value() {
  for supplied_etag in [false, true] {
    let mut router = Router::new();
    router.route(Method::GET, "/", move |_req: Request| async move {
      let mut response = http::Response::builder()
        .header("content-length", "5")
        .header("set-cookie", "first=1; Path=/")
        .header("set-cookie", "second=2; Path=/")
        .header("vary", "Accept-Encoding")
        .header("vary", "Accept-Language")
        .body(TakoBody::from("hello"))
        .unwrap();
      if supplied_etag {
        response
          .headers_mut()
          .insert("etag", "\"hello\"".parse().unwrap());
      }
      response
    });
    router.middleware(ETag::new().into_middleware());

    let request = http::Request::builder()
      .header("if-none-match", "*")
      .body(TakoBody::empty())
      .unwrap();
    let response = router.dispatch(request).await;

    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(
      response
        .headers()
        .get_all("set-cookie")
        .iter()
        .collect::<Vec<_>>(),
      ["first=1; Path=/", "second=2; Path=/"]
    );
    assert_eq!(
      response
        .headers()
        .get_all("vary")
        .iter()
        .collect::<Vec<_>>(),
      ["Accept-Encoding", "Accept-Language"]
    );
    assert!(response.headers().contains_key("etag"));
    assert!(!response.headers().contains_key("content-length"));
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
async fn last_modified_alone_can_answer_a_conditional_request() {
  for method in [Method::GET, Method::HEAD] {
    let mut router = Router::new();
    router.route(method.clone(), "/", |_req: Request| async {
      http::Response::builder()
        .header("last-modified", "Sun, 06 Nov 1994 08:49:37 GMT")
        .body(TakoBody::from("hello"))
        .unwrap()
    });
    router.middleware(ETag::new().max_bytes(0).into_middleware());

    for (date, expected) in [
      ("Sun, 06 Nov 1994 08:49:37 GMT", StatusCode::NOT_MODIFIED),
      ("Sunday, 06-Nov-94 08:49:38 GMT", StatusCode::NOT_MODIFIED),
      ("Sun, 06 Nov 1994 08:49:36 GMT", StatusCode::OK),
      ("invalid-date", StatusCode::OK),
    ] {
      let request = http::Request::builder()
        .method(method.clone())
        .header("if-modified-since", date)
        .body(TakoBody::empty())
        .unwrap();
      let response = router.dispatch(request).await;
      assert_eq!(response.status(), expected, "{method}: {date}");
      assert_eq!(
        response.headers()["last-modified"],
        "Sun, 06 Nov 1994 08:49:37 GMT"
      );
    }
  }
}

#[tokio::test]
async fn if_none_match_takes_precedence_over_if_modified_since() {
  for supplied_etag in [false, true] {
    let mut router = Router::new();
    router.route(Method::GET, "/", move |_req: Request| async move {
      let mut response = http::Response::builder()
        .header("last-modified", "Sun, 06 Nov 1994 08:49:37 GMT")
        .body(TakoBody::from("hello"))
        .unwrap();
      if supplied_etag {
        response
          .headers_mut()
          .insert("etag", "\"hello\"".parse().unwrap());
      }
      response
    });
    router.middleware(ETag::new().into_middleware());

    let request = http::Request::builder()
      .header("if-none-match", "\"different\"")
      .header("if-modified-since", "Sun, 06 Nov 1994 08:49:37 GMT")
      .body(TakoBody::empty())
      .unwrap();
    let response = router.dispatch(request).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
      response.into_body().collect().await.unwrap().to_bytes(),
      "hello"
    );
  }
}

#[tokio::test]
async fn malformed_modification_dates_do_not_validate_cached_content() {
  let mut router = Router::new();
  router.route(Method::GET, "/", |_req: Request| async {
    http::Response::builder()
      .header("etag", "\"hello\"")
      .header("last-modified", "invalid-date")
      .body(TakoBody::from("hello"))
      .unwrap()
  });
  router.middleware(ETag::new().into_middleware());

  let request = http::Request::builder()
    .header("if-modified-since", "invalid-date")
    .body(TakoBody::empty())
    .unwrap();
  let response = router.dispatch(request).await;
  assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn repeated_modification_dates_do_not_validate_cached_content() {
  let mut router = Router::new();
  router.get("/", |_req: Request| async {
    http::Response::builder()
      .header("last-modified", "Sun, 06 Nov 1994 08:49:37 GMT")
      .body(TakoBody::from("hello"))
      .unwrap()
  });
  router.middleware(ETag::new().max_bytes(0).into_middleware());
  let request = http::Request::builder()
    .header("if-modified-since", "Sun, 06 Nov 1994 08:49:37 GMT")
    .header("if-modified-since", "Sun, 06 Nov 1994 08:49:36 GMT")
    .body(TakoBody::empty())
    .unwrap();
  let response = router.dispatch(request).await;
  assert_eq!(response.status(), StatusCode::OK);
  assert_eq!(
    response.into_body().collect().await.unwrap().to_bytes(),
    "hello"
  );
}
