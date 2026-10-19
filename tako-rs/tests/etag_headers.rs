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

#[tokio::test]
async fn repeated_if_none_match_fields_are_all_compared() {
  for supplied_etag in [false, true] {
    let etag = if supplied_etag {
      "\"hello\""
    } else {
      "W/\"2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824\""
    };
    let mut router = Router::new();
    router.get("/", move |_req: Request| async move {
      let mut response = http::Response::new(TakoBody::from("hello"));
      if supplied_etag {
        response.headers_mut().insert("etag", etag.parse().unwrap());
      }
      response
    });
    router.middleware(ETag::new().into_middleware());

    let request = http::Request::builder()
      .header("if-none-match", "\"different\"")
      .header("if-none-match", etag)
      .body(TakoBody::empty())
      .unwrap();
    let response = router.dispatch(request).await;

    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(response.headers()["etag"], etag);
  }
}

#[tokio::test]
async fn if_none_match_keeps_commas_inside_opaque_tags() {
  let mut router = Router::new();
  router.get("/", |_req: Request| async {
    http::Response::builder()
      .header("etag", "W/\"hello,world\"")
      .body(TakoBody::from("hello"))
      .unwrap()
  });
  router.middleware(ETag::new().into_middleware());

  for (condition, expected) in [
    ("\"hello,world\"", StatusCode::NOT_MODIFIED),
    ("\"other\", \"hello,world\"", StatusCode::NOT_MODIFIED),
    ("W/\"hello,world\", \"other\"", StatusCode::NOT_MODIFIED),
    ("\"hello\", \"world\"", StatusCode::OK),
  ] {
    let request = http::Request::builder()
      .header("if-none-match", condition)
      .body(TakoBody::empty())
      .unwrap();
    let response = router.dispatch(request).await;
    assert_eq!(response.status(), expected, "{condition}");
  }
}

#[tokio::test]
async fn wildcard_if_none_match_does_not_require_hashing_oversized_body() {
  for method in [Method::GET, Method::HEAD] {
    let mut router = Router::new();
    router.route(method.clone(), "/", |_req: Request| async {
      http::Response::builder()
        .header("content-length", "5")
        .header("set-cookie", "first=1; Path=/")
        .header("set-cookie", "second=2; Path=/")
        .body(TakoBody::from("hello"))
        .unwrap()
    });
    router.middleware(ETag::new().max_bytes(0).into_middleware());

    let request = http::Request::builder()
      .method(method.clone())
      .header("if-none-match", "*")
      .body(TakoBody::empty())
      .unwrap();
    let response = router.dispatch(request).await;

    assert_eq!(response.status(), StatusCode::NOT_MODIFIED, "{method}");
    assert_eq!(
      response
        .headers()
        .get_all("set-cookie")
        .iter()
        .collect::<Vec<_>>(),
      ["first=1; Path=/", "second=2; Path=/"]
    );
    assert!(!response.headers().contains_key("etag"));
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
async fn oversized_body_without_a_matching_condition_passes_through() {
  let mut router = Router::new();
  router.get("/", |_req: Request| async { "hello" });
  router.middleware(ETag::new().max_bytes(0).into_middleware());

  let request = http::Request::builder()
    .header("if-none-match", "\"hello\"")
    .body(TakoBody::empty())
    .unwrap();
  let response = router.dispatch(request).await;

  assert_eq!(response.status(), StatusCode::OK);
  assert!(!response.headers().contains_key("etag"));
  assert_eq!(
    response.into_body().collect().await.unwrap().to_bytes(),
    "hello"
  );
}

#[tokio::test]
async fn wildcard_if_none_match_keeps_other_methods_and_statuses() {
  for (method, status) in [
    (Method::GET, StatusCode::NOT_FOUND),
    (Method::GET, StatusCode::NO_CONTENT),
    (Method::HEAD, StatusCode::BAD_REQUEST),
    (Method::POST, StatusCode::OK),
    (Method::PUT, StatusCode::OK),
  ] {
    let mut router = Router::new();
    router.route(method.clone(), "/", move |_req: Request| async move {
      http::Response::builder()
        .status(status)
        .body(TakoBody::from("hello"))
        .unwrap()
    });
    router.middleware(ETag::new().max_bytes(0).into_middleware());

    let request = http::Request::builder()
      .method(method.clone())
      .header("if-none-match", "*")
      .body(TakoBody::empty())
      .unwrap();
    let response = router.dispatch(request).await;

    assert_eq!(response.status(), status, "{method}");
    assert!(!response.headers().contains_key("etag"));
  }
}
