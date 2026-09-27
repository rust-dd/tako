#![no_main]

use libfuzzer_sys::fuzz_target;
use tako_rs_extractors::cookie_jar::CookieJar;

fuzz_target!(|data: &[u8]| {
  let Ok(value) = http::HeaderValue::from_bytes(data) else {
    return;
  };
  let mut headers = http::HeaderMap::new();
  headers.insert(http::header::COOKIE, value);
  let jar = CookieJar::from_headers(&headers);
  for cookie in jar.iter() {
    let _ = jar.get(cookie.name());
  }
});
