use cookie::Cookie;
use cookie::Key;
use http::HeaderValue;
use http::header::COOKIE;
use tako_rs_core::body::TakoBody;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::types::Request;
use tako_rs_extractors::cookie_jar::CookieJar;
use tako_rs_extractors::cookie_private::CookiePrivate;
use tako_rs_extractors::cookie_signed::CookieSigned;
use tako_rs_extractors::cookie_signed::KeyRing;

fn request_with_cookies(second: HeaderValue) -> Request {
  let mut req = Request::new(TakoBody::empty());
  req
    .headers_mut()
    .append(COOKIE, HeaderValue::from_static("first=one"));
  req
    .headers_mut()
    .append(COOKIE, HeaderValue::from_bytes(b"invalid=\xff").unwrap());
  req.headers_mut().append(COOKIE, second);
  req
}

#[tokio::test]
async fn plain_cookie_extractors_read_every_cookie_header() {
  let mut req = request_with_cookies(HeaderValue::from_static("second=two; third=three"));
  let jar = CookieJar::from_request(&mut req).await.unwrap();
  assert_eq!(jar.get("first").unwrap().value(), "one");
  assert_eq!(jar.get("second").map(Cookie::value), Some("two"));
  assert_eq!(jar.get("third").map(Cookie::value), Some("three"));

  let (mut parts, _) = req.into_parts();
  let jar = CookieJar::from_request_parts(&mut parts).await.unwrap();
  assert_eq!(jar.get("second").map(Cookie::value), Some("two"));
}

#[tokio::test]
async fn signed_cookie_extractors_verify_later_cookie_headers() {
  let key = Key::generate();
  let mut signed = CookieSigned::new(key.clone());
  signed.add(Cookie::new("session", "signed-value"));
  let value = signed
    .inner()
    .get("session")
    .unwrap()
    .to_string()
    .parse()
    .unwrap();
  let mut req = request_with_cookies(value);
  req.extensions_mut().insert(key.clone());
  let jar = CookieSigned::from_request(&mut req).await.unwrap();
  assert_eq!(jar.get_value("session").as_deref(), Some("signed-value"));

  let (mut parts, _) = req.into_parts();
  parts.extensions.insert(KeyRing::new("active", key));
  let jar = CookieSigned::from_request_parts(&mut parts).await.unwrap();
  assert_eq!(jar.get_value("session").as_deref(), Some("signed-value"));
}

#[tokio::test]
async fn private_cookie_extractors_decrypt_later_cookie_headers() {
  let key = Key::generate();
  let mut private = CookiePrivate::new(key.clone());
  private.add(Cookie::new("session", "secret-value"));
  let value = private
    .inner()
    .get("session")
    .unwrap()
    .to_string()
    .parse()
    .unwrap();
  let mut req = request_with_cookies(value);
  req.extensions_mut().insert(key.clone());
  let jar = CookiePrivate::from_request(&mut req).await.unwrap();
  assert_eq!(jar.get_value("session").as_deref(), Some("secret-value"));

  let (mut parts, _) = req.into_parts();
  parts.extensions.insert(KeyRing::new("active", key));
  let jar = CookiePrivate::from_request_parts(&mut parts).await.unwrap();
  assert_eq!(jar.get_value("session").as_deref(), Some("secret-value"));
}
