use cookie::Cookie;
use cookie::Key;
use http::HeaderValue;
use http::StatusCode;
use http::header::COOKIE;
use http::header::SET_COOKIE;
use http_body_util::BodyExt;
use tako_rs_core::body::TakoBody;
use tako_rs_core::responder::Responder;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;
use tako_rs_extractors::cookie_jar::CookieJar;
use tako_rs_extractors::cookie_private::CookiePrivate;
use tako_rs_extractors::cookie_signed::CookieSigned;

#[tokio::test]
async fn returned_jar_emits_additions_and_removals_without_echoing_request_cookies() {
  let mut router = Router::new();
  router.get("/", |mut jar: CookieJar| async move {
    jar.add(
      Cookie::build(("fresh", "value"))
        .path("/")
        .http_only(true)
        .build(),
    );
    jar.remove("expired");
    jar
  });
  let request = http::Request::builder()
    .uri("/")
    .header(COOKIE, "expired=old; untouched=keep")
    .body(TakoBody::empty())
    .unwrap();
  let response = router.dispatch(request).await;
  assert_eq!(response.status(), StatusCode::OK);
  let cookies = response
    .headers()
    .get_all(SET_COOKIE)
    .iter()
    .map(|value| Cookie::parse(value.to_str().unwrap()).unwrap())
    .collect::<Vec<_>>();
  assert_eq!(cookies.len(), 2);
  let added = cookies
    .iter()
    .find(|cookie| cookie.name() == "fresh")
    .unwrap();
  assert_eq!(added.value(), "value");
  assert_eq!(added.path(), Some("/"));
  assert_eq!(added.http_only(), Some(true));
  let removed = cookies
    .iter()
    .find(|cookie| cookie.name() == "expired")
    .unwrap();
  assert_eq!(removed.value(), "");
  assert_eq!(removed.max_age().unwrap().whole_seconds(), 0);
}

#[tokio::test]
async fn cookie_response_preserves_body_status_and_other_set_cookie_headers() {
  let mut router = Router::new();
  router.get("/", |mut jar: CookieJar| async move {
    jar.add(Cookie::new("second", "two"));
    jar.add(Cookie::new("third", "three"));
    let mut response = (StatusCode::CREATED, "saved").into_response();
    response
      .headers_mut()
      .append(SET_COOKIE, HeaderValue::from_static("first=one"));
    jar.with_response(response)
  });
  let response = router.dispatch(Request::new(TakoBody::empty())).await;
  assert_eq!(response.status(), StatusCode::CREATED);
  let mut cookies = response
    .headers()
    .get_all(SET_COOKIE)
    .iter()
    .map(|value| value.to_str().unwrap())
    .collect::<Vec<_>>();
  cookies.sort_unstable();
  assert_eq!(cookies, ["first=one", "second=two", "third=three"]);
  assert_eq!(
    response.into_body().collect().await.unwrap().to_bytes(),
    "saved"
  );
}

#[test]
fn signed_cookie_response_contains_verified_changes() {
  let key = Key::generate();
  let mut jar = CookieSigned::new(key.clone());
  jar.add(Cookie::new("session", "signed-value"));
  let response = jar.into_response();
  let mut headers = http::HeaderMap::new();
  headers.append(COOKIE, response.headers()[SET_COOKIE].clone());
  let read = CookieSigned::from_headers(&headers, key);
  assert_eq!(read.get_value("session").as_deref(), Some("signed-value"));
}

#[test]
fn private_cookie_response_contains_encrypted_changes() {
  let key = Key::generate();
  let mut jar = CookiePrivate::new(key.clone());
  jar.add(Cookie::new("session", "secret-value"));
  let response = jar.with_response("ok");
  assert!(
    !response.headers()[SET_COOKIE]
      .to_str()
      .unwrap()
      .contains("secret-value")
  );
  let mut headers = http::HeaderMap::new();
  headers.append(COOKIE, response.headers()[SET_COOKIE].clone());
  let read = CookiePrivate::from_headers(&headers, key);
  assert_eq!(read.get_value("session").as_deref(), Some("secret-value"));
}

#[test]
fn cookie_responses_reject_invalid_names_values_and_injected_attributes() {
  for (name, value) in [
    ("", "value"),
    ("name=other", "value"),
    ("name; Domain=example.com", "value"),
    ("name\n", "value"),
    ("café", "value"),
    ("name", "input; Domain=example.com; Path=/; SameSite=None"),
    ("name", "line\r\nvalue"),
    ("name", "comma,value"),
    ("name", "back\\slash"),
    ("name", "has space"),
    ("name", "café"),
  ] {
    let mut jar = CookieJar::new();
    jar.add(Cookie::new(name, value));
    let response = jar.with_response("saved");
    assert_eq!(
      response.status(),
      StatusCode::INTERNAL_SERVER_ERROR,
      "{name:?}={value:?}"
    );
    assert!(!response.headers().contains_key(SET_COOKIE));
  }

  for cookie in [
    Cookie::build(("name", "value"))
      .path("/; Domain=example.com")
      .build(),
    Cookie::build(("name", "value"))
      .domain("example.com; Secure")
      .build(),
  ] {
    let mut jar = CookieJar::new();
    jar.add(cookie);
    let response = jar.into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!response.headers().contains_key(SET_COOKIE));
  }
}

#[test]
fn signed_response_rejects_injection_and_private_response_keeps_encrypted_values_opaque() {
  let value = "café; Domain=example.com; Path=/";
  let key = Key::generate();
  let mut signed = CookieSigned::new(key.clone());
  signed.add(Cookie::new("session", value));
  let response = signed.into_response();
  assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
  assert!(!response.headers().contains_key(SET_COOKIE));

  let mut private = CookiePrivate::new(key.clone());
  private.add(Cookie::new("session", value));
  let response = private.into_response();
  assert_eq!(response.status(), StatusCode::OK);
  let cookie = Cookie::parse(response.headers()[SET_COOKIE].to_str().unwrap()).unwrap();
  assert_eq!(cookie.domain(), None);
  assert_eq!(cookie.path(), None);
  let mut headers = http::HeaderMap::new();
  headers.append(COOKIE, response.headers()[SET_COOKIE].clone());
  assert_eq!(
    CookiePrivate::from_headers(&headers, key)
      .get_value("session")
      .as_deref(),
    Some(value)
  );
}

#[test]
fn cookie_response_preserves_valid_cookie_octets_and_quoted_values() {
  for value in ["", "!#$%&'()*+-./:<=>?@[]^_`{|}~", "\"quoted\""] {
    let mut jar = CookieJar::new();
    jar.add(Cookie::new("name", value));
    let response = jar.into_response();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[SET_COOKIE], format!("name={value}"));
  }
}
