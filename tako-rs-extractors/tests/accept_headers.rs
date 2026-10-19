use http::header::ACCEPT;
use tako_rs_core::body::TakoBody;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::types::Request;
use tako_rs_extractors::accept::Accept;

async fn extract_accept(headers: &[&str]) -> [Accept; 2] {
  let mut request = Request::new(TakoBody::empty());
  for header in headers {
    request
      .headers_mut()
      .append(ACCEPT, header.parse().unwrap());
  }
  let from_request = Accept::from_request(&mut request).await.unwrap();
  let (mut parts, _) = request.into_parts();
  let from_parts = Accept::from_request_parts(&mut parts).await.unwrap();
  [from_request, from_parts]
}

#[tokio::test]
async fn repeated_accept_fields_preserve_specific_quality_exclusions() {
  for (headers, media_type, expected) in [
    (["*/*", "application/json;q=0"], "application/json", false),
    (["text/*", "text/html;q=0"], "text/html", false),
    (
      ["text/plain", "text/plain;format=flowed;q=0"],
      "text/plain;format=flowed",
      false,
    ),
    (["text/*;q=0", "text/html"], "text/html", true),
  ] {
    for parsed in extract_accept(&headers).await {
      assert_eq!(parsed.accepts(media_type), expected, "{headers:?}");
      if !expected {
        assert!(!parsed.prefers(media_type), "{headers:?}");
      }
    }
  }
}

#[tokio::test]
async fn repeated_accept_fields_share_one_stable_quality_order() {
  for parsed in extract_accept(&[
    "text/plain;q=0.2, image/png;q=0.7",
    "application/json;q=0.8, text/html;q=0.7",
  ])
  .await
  {
    assert_eq!(parsed.preferred(), Some("application/json"));
    assert_eq!(
      parsed.types(),
      ["application/json", "image/png", "text/html", "text/plain"]
    );
    assert!(parsed.prefers("application/json"));
    assert!(!parsed.prefers("image/png"));
  }
}

#[tokio::test]
async fn quoted_commas_remain_media_parameters() {
  for parsed in extract_accept(&[r#"text/plain;format="a,b";q=0.9, application/json;q=0.3"#]).await
  {
    assert_eq!(parsed.preferred(), Some(r#"text/plain;format="a,b""#));
    assert_eq!(
      parsed.types(),
      [r#"text/plain;format="a,b""#, "application/json"]
    );
    assert!(parsed.accepts(r#"text/plain;format="a,b""#));
    assert!(parsed.prefers(r#"text/plain;format="a,b""#));
    assert!(!parsed.accepts("text/plain"));
  }
}

#[tokio::test]
async fn quoted_commas_preserve_parameter_specific_exclusions() {
  for parsed in extract_accept(&[r#"text/plain;format="a,b";q=0, text/plain"#]).await {
    assert!(!parsed.accepts(r#"text/plain;format="a,b""#));
    assert!(!parsed.prefers(r#"text/plain;format="a,b""#));
    assert!(parsed.accepts("text/plain"));
    assert_eq!(parsed.types(), ["text/plain"]);
  }
}

#[tokio::test]
async fn escaped_quotes_do_not_expose_embedded_media_ranges() {
  for parsed in
    extract_accept(&[r#"text/plain;format="a\",application/json,b", text/html;q=0.5"#]).await
  {
    assert!(!parsed.accepts("application/json"));
    assert!(parsed.accepts("text/html"));
  }
}

#[tokio::test]
async fn escaped_backslashes_allow_following_media_ranges() {
  for parsed in extract_accept(&[r#"text/plain;format="a\\", application/json;q=0.5"#]).await {
    assert!(parsed.accepts("application/json"));
    assert_eq!(parsed.types().len(), 2);
  }
}

#[tokio::test]
async fn unterminated_quotes_do_not_expose_embedded_media_ranges() {
  for header in [
    r#"text/plain;format="unterminated,application/json"#,
    r#"text/plain;format="unterminated\",application/json"#,
  ] {
    for parsed in extract_accept(&[header]).await {
      assert_eq!(parsed.preferred(), None, "{header}");
      assert!(parsed.types().is_empty(), "{header}");
      assert!(!parsed.accepts("application/json"), "{header}");
    }
  }
}

#[tokio::test]
async fn malformed_fields_do_not_discard_later_valid_fields() {
  for header in ["not-a-media-type", r#"text/plain;format="unterminated"#] {
    for parsed in extract_accept(&[header, "application/json;q=0.8"]).await {
      assert_eq!(parsed.preferred(), Some("application/json"));
      assert_eq!(parsed.types(), ["application/json"]);
      assert!(parsed.prefers("application/json"));
    }
  }
}
