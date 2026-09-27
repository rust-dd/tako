use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use http::HeaderMap;
use http::Method;
use http::StatusCode;
use http::Version;
use http::header;
use sha1::Digest;
use sha1::Sha1;
use tako_rs_core::body::TakoBody;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

pub(crate) fn accept(request: &Request) -> Result<String, StatusCode> {
  if request.method() != Method::GET {
    return Err(StatusCode::METHOD_NOT_ALLOWED);
  }
  if request.version() != Version::HTTP_11 {
    return Err(StatusCode::BAD_REQUEST);
  }
  let headers = request.headers();
  let host = single(headers, header::HOST).ok_or(StatusCode::BAD_REQUEST)?;
  if host.contains('@') || host.parse::<http::uri::Authority>().is_err() {
    return Err(StatusCode::BAD_REQUEST);
  }
  if !has_token(headers, header::UPGRADE, "websocket")
    || !has_token(headers, header::CONNECTION, "upgrade")
  {
    return Err(StatusCode::BAD_REQUEST);
  }
  let version = single(headers, header::SEC_WEBSOCKET_VERSION).ok_or(StatusCode::BAD_REQUEST)?;
  if version != "13" {
    return Err(StatusCode::UPGRADE_REQUIRED);
  }
  let key = single(headers, header::SEC_WEBSOCKET_KEY).ok_or(StatusCode::BAD_REQUEST)?;
  if key.len() != 24
    || !STANDARD
      .decode(key)
      .is_ok_and(|decoded| decoded.len() == 16)
  {
    return Err(StatusCode::BAD_REQUEST);
  }
  let mut hash = Sha1::new();
  hash.update(key.as_bytes());
  hash.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
  Ok(STANDARD.encode(hash.finalize()))
}

pub(crate) fn rejection(status: StatusCode) -> Response {
  let mut response = http::Response::builder().status(status);
  if status == StatusCode::UPGRADE_REQUIRED {
    response = response.header(header::SEC_WEBSOCKET_VERSION, "13");
  }
  if status == StatusCode::METHOD_NOT_ALLOWED {
    response = response.header(header::ALLOW, "GET");
  }
  response
    .body(TakoBody::empty())
    .expect("valid handshake rejection")
}

fn single(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
  let mut values = headers.get_all(name).iter();
  let value = values.next()?.to_str().ok()?.trim();
  (values.next().is_none() && !value.is_empty()).then_some(value)
}

fn has_token(headers: &HeaderMap, name: header::HeaderName, token: &str) -> bool {
  headers
    .get_all(name)
    .iter()
    .filter_map(|value| value.to_str().ok())
    .flat_map(|value| value.split(','))
    .any(|value| value.trim().eq_ignore_ascii_case(token))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn valid() -> Request {
    http::Request::builder()
      .uri("/ws")
      .header(header::HOST, "example.com")
      .header(header::UPGRADE, "WebSocket")
      .header(header::CONNECTION, "keep-alive, Upgrade")
      .header(header::SEC_WEBSOCKET_VERSION, "13")
      .header(header::SEC_WEBSOCKET_KEY, "dGhlIHNhbXBsZSBub25jZQ==")
      .body(TakoBody::empty())
      .unwrap()
  }

  #[test]
  fn valid_handshake_matches_the_protocol_example() {
    assert_eq!(accept(&valid()).unwrap(), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    let mut request = valid();
    request
      .headers_mut()
      .insert(header::CONNECTION, "keep-alive".parse().unwrap());
    request
      .headers_mut()
      .append(header::CONNECTION, "upgrade".parse().unwrap());
    assert!(accept(&request).is_ok());
  }

  #[test]
  fn missing_duplicate_and_malformed_headers_are_rejected() {
    for name in [
      header::HOST,
      header::UPGRADE,
      header::CONNECTION,
      header::SEC_WEBSOCKET_VERSION,
      header::SEC_WEBSOCKET_KEY,
    ] {
      let mut request = valid();
      request.headers_mut().remove(name);
      assert_eq!(accept(&request).unwrap_err(), StatusCode::BAD_REQUEST);
    }
    for key in [
      "invalid",
      "YQ==",
      "AAAAAAAAAAAAAAAAAAAAAA=A",
      "AAAAAAAAAAAAAAAAAAAAAA==, AAAAAAAAAAAAAAAAAAAAAA==",
    ] {
      let mut request = valid();
      request
        .headers_mut()
        .insert(header::SEC_WEBSOCKET_KEY, key.parse().unwrap());
      assert_eq!(accept(&request).unwrap_err(), StatusCode::BAD_REQUEST);
    }
    let mut request = valid();
    request.headers_mut().append(
      header::SEC_WEBSOCKET_KEY,
      "AAAAAAAAAAAAAAAAAAAAAA==".parse().unwrap(),
    );
    assert_eq!(accept(&request).unwrap_err(), StatusCode::BAD_REQUEST);
  }

  #[test]
  fn unsupported_versions_and_methods_explain_the_supported_handshake() {
    let mut request = valid();
    request
      .headers_mut()
      .insert(header::SEC_WEBSOCKET_VERSION, "12".parse().unwrap());
    let response = rejection(accept(&request).unwrap_err());
    assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
    assert_eq!(response.headers()[header::SEC_WEBSOCKET_VERSION], "13");
    *request.method_mut() = Method::POST;
    let response = rejection(accept(&request).unwrap_err());
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers()[header::ALLOW], "GET");
    *request.method_mut() = Method::GET;
    *request.version_mut() = Version::HTTP_2;
    assert_eq!(accept(&request).unwrap_err(), StatusCode::BAD_REQUEST);
  }
}
