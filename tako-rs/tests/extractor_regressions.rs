use http::StatusCode;
use http_body_util::BodyExt;
use tako::body::TakoBody;
use tako::extractors::FromRequest;
use tako::extractors::FromRequestParts;
use tako::extractors::accept::Accept;
use tako::extractors::uri_parts::OriginalUri;
#[cfg(feature = "multipart")]
use tako::responder::Responder;
use tako::router::Router;
use tako::types::Request;

async fn accept(header: &str) -> Accept {
  let mut req = http::Request::builder()
    .header("accept", header)
    .body(TakoBody::empty())
    .unwrap();
  Accept::from_request(&mut req).await.unwrap()
}

#[tokio::test]
async fn accept_zero_quality_exclusions_override_less_specific_ranges() {
  for (header, media_type, expected) in [
    ("application/json;q=0", "application/json", false),
    ("*/*, application/json;q=0", "application/json", false),
    ("text/*, text/html;q=0", "text/html", false),
    ("*/*, text/*;q=0", "text/plain", false),
    ("*/*;q=0, text/plain;q=0.5", "text/plain", true),
    ("text/*;q=0, text/html;q=1", "text/html", true),
    ("*/*, application/json; q=0", "application/json", false),
    (
      "text/plain, text/plain;format=flowed;q=0",
      "text/plain;format=flowed",
      false,
    ),
    (
      "text/plain, text/plain;format=flowed;q=0",
      "text/plain;format=fixed",
      true,
    ),
    (
      "text/plain;q=0, text/plain;format=flowed",
      "text/plain;format=flowed",
      true,
    ),
  ] {
    let parsed = accept(header).await;
    assert_eq!(
      parsed.accepts(media_type),
      expected,
      "{header} / {media_type}"
    );
    if !expected {
      assert!(!parsed.prefers(media_type), "{header} / {media_type}");
    }
  }
}

#[tokio::test]
async fn accept_zero_quality_entries_are_not_offered_as_preferences() {
  let parsed = accept("application/json;q=0, text/html;q=0").await;
  assert_eq!(parsed.preferred(), None);
  assert_eq!(parsed.types(), Vec::<&str>::new());
  assert!(!parsed.prefers("application/json"));
}

#[tokio::test]
async fn accept_preferences_preserve_media_parameters_without_quality() {
  for (header, expected) in [
    (
      "text/plain;q=0, text/plain;format=flowed;q=1",
      "text/plain;format=flowed",
    ),
    (
      "text/plain;q=0, text/plain;q=1;format=flowed",
      "text/plain;format=flowed",
    ),
    (
      "text/plain;q=0, text/plain;format=\"flowed value\";q=1",
      "text/plain;format=\"flowed value\"",
    ),
    (
      "text/plain;q=0, text/plain;format=\"semi;colon\";q=1",
      "text/plain;format=\"semi;colon\"",
    ),
  ] {
    let parsed = accept(header).await;
    assert_eq!(parsed.preferred(), Some(expected));
    assert_eq!(parsed.types(), [expected]);
    assert!(parsed.accepts(parsed.preferred().unwrap()));
    assert!(parsed.prefers(parsed.preferred().unwrap()));
    assert!(!parsed.accepts("text/plain"));
  }
}

#[tokio::test]
async fn accept_parts_extraction_obeys_zero_quality() {
  let (mut parts, ()) = http::Request::builder()
    .header("accept", "*/*, application/json;q=0")
    .body(())
    .unwrap()
    .into_parts();
  let parsed = Accept::from_request_parts(&mut parts).await.unwrap();
  assert!(!parsed.accepts("application/json"));
}

#[tokio::test]
async fn basic_auth_scheme_is_case_insensitive_but_credentials_are_unchanged() {
  use tako::extractors::basic::Basic;

  for scheme in ["basic", "BASIC", "bAsIc"] {
    let value = format!("{scheme} QWxpY2U6U2VDcmV0");
    let mut req = http::Request::builder()
      .header("authorization", &value)
      .body(TakoBody::empty())
      .unwrap();
    let basic = Basic::from_request(&mut req).await.unwrap();
    assert_eq!(basic.username, "Alice");
    assert_eq!(basic.password, "SeCret");
    assert_eq!(basic.raw, value);
    let (mut parts, _) = req.into_parts();
    let basic = Basic::from_request_parts(&mut parts).await.unwrap();
    assert_eq!(basic.password, "SeCret");
  }
}

#[tokio::test]
async fn bearer_auth_scheme_is_case_insensitive_but_tokens_are_unchanged() {
  use tako::extractors::bearer::Bearer;

  for scheme in ["bearer", "BEARER", "bEaReR"] {
    let value = format!("{scheme} SeCrEt-token");
    let mut req = http::Request::builder()
      .header("authorization", &value)
      .body(TakoBody::empty())
      .unwrap();
    let bearer = Bearer::from_request(&mut req).await.unwrap();
    assert_eq!(bearer.token, "SeCrEt-token");
    assert_eq!(bearer.with_bearer, value);
    let (mut parts, _) = req.into_parts();
    let bearer = Bearer::from_request_parts(&mut parts).await.unwrap();
    assert_eq!(bearer.token, "SeCrEt-token");
  }
}

#[tokio::test]
async fn original_uri_survives_nested_middleware_rewrites() {
  let mut child = Router::new();
  child.get("/users", |original: OriginalUri, req: Request| async move {
    format!("{}|{}", original.0, req.uri())
  });
  child.middleware(|mut req, next| async move {
    *req.uri_mut() = "/users?inside=1".parse().unwrap();
    next.run(req).await
  });
  let mut router = Router::new();
  router.nest("/api", child);
  let request = http::Request::builder()
    .uri("/api/users?outside=2")
    .body(TakoBody::empty())
    .unwrap();
  let response = router.dispatch(request).await;
  assert_eq!(response.status(), StatusCode::OK);
  let body = response.into_body().collect().await.unwrap().to_bytes();
  assert_eq!(body, "/api/users?outside=2|/users?inside=1");
}

#[tokio::test]
async fn original_uri_survives_redispatch_after_a_handler_rewrite() {
  let mut inner = Router::new();
  inner.get("/inside", |original: OriginalUri| async move {
    original.0.to_string()
  });
  let forwarded = std::sync::Arc::new(std::sync::Mutex::new(None));
  let outgoing = forwarded.clone();
  let mut outer = Router::new();
  outer.get("/outside", move |mut req: Request| {
    *req.uri_mut() = "/inside?rewritten=1".parse().unwrap();
    *outgoing.lock().unwrap() = Some(req);
    std::future::ready(StatusCode::OK)
  });
  let request = http::Request::builder()
    .uri("/outside?original=2")
    .body(TakoBody::empty())
    .unwrap();
  assert_eq!(outer.dispatch(request).await.status(), StatusCode::OK);
  let request = forwarded.lock().unwrap().take().unwrap();
  let response = inner.dispatch(request).await;
  assert_eq!(response.status(), StatusCode::OK);
  assert_eq!(
    response.into_body().collect().await.unwrap().to_bytes(),
    "/outside?original=2"
  );
}

#[cfg(feature = "multipart")]
#[tokio::test]
async fn untyped_multipart_enforces_router_max_parts() {
  use tako::extractors::multipart::MultipartConfig;
  use tako::extractors::multipart::TakoMultipart;

  async fn upload(mut req: Request) -> tako::types::Response {
    let TakoMultipart(mut multipart) = TakoMultipart::from_request(&mut req).await.unwrap();
    loop {
      match multipart.next_field().await {
        Ok(Some(field)) => {
          field.bytes().await.unwrap();
        }
        Ok(None) => return StatusCode::OK.into_response(),
        Err(error) => return error.into_response(),
      }
    }
  }

  let mut router = Router::new();
  router.with_state(MultipartConfig::new().max_parts(1));
  router.post("/upload", upload);
  let request = http::Request::builder()
    .method(http::Method::POST)
    .uri("/upload")
    .header("content-type", "multipart/form-data; boundary=X")
    .body(TakoBody::from(
      "--X\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\n1\r\n\
       --X\r\nContent-Disposition: form-data; name=\"b\"\r\n\r\n2\r\n--X--\r\n",
    ))
    .unwrap();
  assert_eq!(
    router.dispatch(request).await.status(),
    StatusCode::PAYLOAD_TOO_LARGE
  );
}

#[cfg(feature = "multipart")]
#[tokio::test]
async fn untyped_multipart_rejects_disallowed_content_types() {
  use tako::extractors::multipart::MultipartConfig;
  use tako::extractors::multipart::TakoMultipart;

  for (content_type, permitted) in [
    ("image/png", true),
    ("image/png; charset=binary", true),
    ("image/pngx", false),
    ("text/plain", false),
  ] {
    let body = format!(
      "--X\r\nContent-Disposition: form-data; name=\"file\"\r\nContent-Type: {content_type}\r\n\r\ndata\r\n--X--\r\n"
    );
    let mut req = http::Request::builder()
      .header("content-type", "multipart/form-data; boundary=X")
      .body(TakoBody::from(body))
      .unwrap();
    req
      .extensions_mut()
      .insert(MultipartConfig::new().allowed_content_types(["image/png"]));
    let TakoMultipart(mut multipart) = TakoMultipart::from_request(&mut req).await.unwrap();
    match multipart.next_field().await {
      Ok(Some(_)) => assert!(permitted, "{content_type}"),
      Err(error) => {
        assert!(!permitted, "{content_type}");
        assert_eq!(
          error.into_response().status(),
          StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
      }
      Ok(None) => panic!("missing field"),
    }
  }
}

#[cfg(feature = "multipart")]
#[tokio::test]
async fn untyped_multipart_inner_stream_enforces_count_boundaries_and_cannot_resume_after_limit() {
  use tako::extractors::multipart::MultipartConfig;
  use tako::extractors::multipart::MultipartError;
  use tako::extractors::multipart::TakoMultipart;

  for (limit, count, expected_count, rejected) in [
    (Some(0), 0, 0, false),
    (Some(0), 1, 0, true),
    (Some(1), 1, 1, false),
    (Some(1), 3, 1, true),
    (None, 3, 3, false),
  ] {
    let mut body = String::new();
    for index in 0..count {
      body.push_str(&format!(
        "--X\r\nContent-Disposition: form-data; name=\"field{index}\"\r\n\r\nvalue\r\n"
      ));
    }
    body.push_str("--X--\r\n");
    let mut req = http::Request::builder()
      .header("content-type", "multipart/form-data; boundary=X")
      .body(TakoBody::from(body))
      .unwrap();
    let mut config = MultipartConfig::new();
    config.max_parts = limit;
    req.extensions_mut().insert(config);
    let mut multipart = TakoMultipart::from_request(&mut req)
      .await
      .unwrap()
      .into_inner();
    for expected_index in 0..expected_count {
      let (index, field) = multipart.next_field_with_idx().await.unwrap().unwrap();
      assert_eq!(index, expected_index);
      assert_eq!(field.text().await.unwrap(), "value");
    }
    let result = std::future::poll_fn(|cx| multipart.poll_next_field(cx)).await;
    if rejected {
      assert!(matches!(result, Err(MultipartError::TooManyParts)));
      assert!(matches!(
        multipart.next_field().await,
        Err(MultipartError::TooManyParts)
      ));
    } else {
      assert!(result.unwrap().is_none());
    }
  }
}
