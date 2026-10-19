use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use http::HeaderMap;
use http::Method;
use http::StatusCode;
use http::Version;
use http::header;
use sha1::Digest;
use sha1::Sha1;

use crate::body::TakoBody;
use crate::types::Request;
use crate::types::Response;

/// Validates an RFC 6455 upgrade request and returns its `Sec-WebSocket-Accept` value.
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

/// The response for a rejected upgrade, naming what the server supports.
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

/// The first of the server's `protocols` that the client offers.
pub(crate) fn negotiate_subprotocol(
  headers: &HeaderMap,
  protocols: &[&'static str],
) -> Option<&'static str> {
  if protocols.is_empty() {
    return None;
  }
  let offered: Vec<&str> = headers
    .get(header::SEC_WEBSOCKET_PROTOCOL)?
    .to_str()
    .ok()?
    .split(',')
    .map(str::trim)
    .collect();
  protocols
    .iter()
    .copied()
    .find(|protocol| offered.contains(protocol))
}

/// Whether the request's `Origin` is on the allow-list; without a list every origin passes.
pub(crate) fn origin_allowed(headers: &HeaderMap, allowed: Option<&[String]>) -> bool {
  let Some(allowed) = allowed else {
    return true;
  };
  let Some(origin) = headers
    .get(header::ORIGIN)
    .and_then(|value| value.to_str().ok())
  else {
    return false;
  };
  let observed = normalize_origin(origin);
  !observed.is_empty()
    && allowed
      .iter()
      .any(|entry| normalize_origin(entry) == observed)
}

/// Scheme, host and non-default port of an origin, lowercased; empty when it is not one.
fn normalize_origin(raw: &str) -> String {
  let raw = raw.trim();
  if raw.is_empty() || raw.eq_ignore_ascii_case("null") {
    return String::new();
  }
  let Ok(url) = url::Url::parse(raw) else {
    return String::new();
  };
  // An Origin never carries credentials.
  if !url.username().is_empty() || url.password().is_some() {
    return String::new();
  }
  let scheme = url.scheme().to_ascii_lowercase();
  let Some(host) = url.host_str() else {
    return String::new();
  };
  let host = host.to_ascii_lowercase();
  let port = url.port();
  let default = matches!(
    (scheme.as_str(), port),
    ("http" | "ws", Some(80)) | ("https" | "wss", Some(443))
  );
  match port {
    Some(port) if !default => format!("{scheme}://{host}:{port}"),
    _ => format!("{scheme}://{host}"),
  }
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
  use http::HeaderMap;
  use http::HeaderValue;
  use http::Method;
  use http::StatusCode;
  use http::Version;
  use http::header;

  use super::accept;
  use super::negotiate_subprotocol;
  use super::normalize_origin;
  use super::origin_allowed;
  use super::rejection;
  use crate::body::TakoBody;
  use crate::types::Request;

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

  #[test]
  fn the_first_server_protocol_the_client_offers_wins() {
    let mut headers = HeaderMap::new();
    assert_eq!(negotiate_subprotocol(&headers, &["chat"]), None);
    headers.insert(
      header::SEC_WEBSOCKET_PROTOCOL,
      HeaderValue::from_static("superchat, chat"),
    );
    assert_eq!(
      negotiate_subprotocol(&headers, &["chat", "superchat"]),
      Some("chat")
    );
    assert_eq!(negotiate_subprotocol(&headers, &["v2"]), None);
    assert_eq!(negotiate_subprotocol(&headers, &[]), None);
  }

  #[test]
  fn origins_are_checked_only_against_an_allow_list() {
    let allowed = vec!["https://example.com".to_string()];
    let mut headers = HeaderMap::new();
    assert!(origin_allowed(&headers, None));
    assert!(!origin_allowed(&headers, Some(&allowed)));
    headers.insert(
      header::ORIGIN,
      HeaderValue::from_static("HTTPS://Example.com:443"),
    );
    assert!(origin_allowed(&headers, Some(&allowed)));
    headers.insert(
      header::ORIGIN,
      HeaderValue::from_static("https://evil.example"),
    );
    assert!(!origin_allowed(&headers, Some(&allowed)));
  }

  #[test]
  fn normalize_origin_lowercases_scheme_and_host() {
    assert_eq!(
      normalize_origin("HTTPS://Example.COM"),
      "https://example.com"
    );
  }

  #[test]
  fn normalize_origin_strips_default_ports() {
    assert_eq!(
      normalize_origin("http://example.com:80"),
      "http://example.com"
    );
    assert_eq!(
      normalize_origin("https://example.com:443"),
      "https://example.com"
    );
  }

  #[test]
  fn normalize_origin_keeps_nondefault_ports() {
    assert_eq!(
      normalize_origin("https://example.com:8443"),
      "https://example.com:8443"
    );
  }

  #[test]
  fn normalize_origin_rejects_malformed_or_null() {
    assert_eq!(normalize_origin(""), "");
    assert_eq!(normalize_origin("null"), "");
    assert_eq!(normalize_origin("not-an-origin"), "");
  }

  #[test]
  fn normalize_origin_ignores_trailing_path() {
    assert_eq!(
      normalize_origin("https://example.com/path?x=1"),
      "https://example.com"
    );
  }
}
