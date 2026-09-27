#![no_main]

use libfuzzer_sys::fuzz_target;
use tako_rs_extractors::ipaddr::IpAddr;
use tako_rs_extractors::ipaddr::IpAddrConfig;

fuzz_target!(|data: &[u8]| {
  let Ok(value) = http::HeaderValue::from_bytes(data) else {
    return;
  };
  let mut extensions = http::Extensions::new();
  extensions.insert("127.0.0.1:8080".parse::<std::net::SocketAddr>().unwrap());
  extensions.insert(IpAddrConfig::new().trust("127.0.0.1".parse().unwrap()));
  for header in ["forwarded", "x-forwarded-for"] {
    let mut headers = http::HeaderMap::new();
    headers.insert(header, value.clone());
    assert!(IpAddr::resolve(&extensions, &headers).is_ok());
  }
});
