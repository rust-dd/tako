//! Redirect response utilities for handlers.
//!
//! This module provides a small helper type and constructors to build
//! HTTP redirect responses from handlers. Example:
//!
//! ```rust
//! use tako::{redirect, responder::Responder};
//!
//! async fn go_home() -> impl Responder {
//!     redirect::found("/")
//! }
//!
//! async fn login_redirect() -> impl Responder {
//!     // Preserve method (307) or change to GET (303) depending on needs
//!     redirect::temporary("/login")
//! }
//! ```

use http::StatusCode;
use http::header::LOCATION;

use crate::body::TakoBody;
use crate::responder::Responder;
use crate::types::Response;

/// A redirect response builder that implements `Responder`.
///
/// Use the constructors like [`found`], [`see_other`], [`temporary`], or [`permanent`]
/// to create redirects with appropriate HTTP status codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
  status: StatusCode,
  location: String,
}

impl Redirect {
  /// Create a redirect with a custom status code.
  #[inline]
  #[must_use]
  pub fn with_status(location: impl Into<String>, status: StatusCode) -> Self {
    Self {
      status,
      location: location.into(),
    }
  }

  /// 302 Found (common temporary redirect).
  #[inline]
  #[must_use]
  pub fn found(location: impl Into<String>) -> Self {
    Self::with_status(location, StatusCode::FOUND)
  }

  /// 303 See Other (commonly used after POST to redirect to a GET page).
  #[inline]
  #[must_use]
  pub fn see_other(location: impl Into<String>) -> Self {
    Self::with_status(location, StatusCode::SEE_OTHER)
  }

  /// 307 Temporary Redirect (preserves the HTTP method).
  #[inline]
  #[must_use]
  pub fn temporary(location: impl Into<String>) -> Self {
    Self::with_status(location, StatusCode::TEMPORARY_REDIRECT)
  }

  /// 301 Moved Permanently.
  #[inline]
  #[must_use]
  pub fn permanent_moved(location: impl Into<String>) -> Self {
    Self::with_status(location, StatusCode::MOVED_PERMANENTLY)
  }

  /// 308 Permanent Redirect.
  #[inline]
  #[must_use]
  pub fn permanent(location: impl Into<String>) -> Self {
    Self::with_status(location, StatusCode::PERMANENT_REDIRECT)
  }
}

impl Responder for Redirect {
  /// Builds the redirect response.
  ///
  /// The `Location` header is constructed via the fallible
  /// [`http::HeaderValue::try_from`], not via `.unwrap()`, so that
  /// caller-supplied locations containing CR / LF / NUL bytes (which would
  /// otherwise be a HTTP response-splitting / open-redirect vector) cannot
  /// turn a redirect into a panic. Malformed locations yield a
  /// `500 Internal Server Error` with an explanatory body instead.
  fn into_response(self) -> Response {
    let Ok(value) = http::HeaderValue::try_from(self.location.as_str()) else {
      let mut resp = http::Response::new(TakoBody::from(
        "redirect location contains invalid header characters",
      ));
      *resp.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
      return resp;
    };
    let mut resp = http::Response::new(TakoBody::empty());
    *resp.status_mut() = self.status;
    resp.headers_mut().insert(LOCATION, value);
    resp
  }
}

/// Shorthand for a 302 Found redirect.
pub fn found(location: impl Into<String>) -> Redirect {
  Redirect::found(location)
}

/// Shorthand for a 303 See Other redirect.
pub fn see_other(location: impl Into<String>) -> Redirect {
  Redirect::see_other(location)
}

/// Shorthand for a 307 Temporary Redirect.
pub fn temporary(location: impl Into<String>) -> Redirect {
  Redirect::temporary(location)
}

/// Shorthand for a 301 Moved Permanently.
pub fn permanent_moved(location: impl Into<String>) -> Redirect {
  Redirect::permanent_moved(location)
}

/// Shorthand for a 308 Permanent Redirect.
pub fn permanent(location: impl Into<String>) -> Redirect {
  Redirect::permanent(location)
}

/// Extracts the host portion (without port) from a `Host` header and validates
/// it as a syntactically well-formed authority. Returns `None` for missing,
/// malformed, or empty values — including anything containing CR/LF or
/// scheme-like prefixes (`javascript:`, `data:`, …) that could be smuggled
/// into a redirect's `Location` header.
fn validate_host(header_value: &str) -> Option<String> {
  let trimmed = header_value.trim();
  if trimmed.is_empty() {
    return None;
  }
  // Reject CR / LF / NUL / whitespace within the value — these would enable
  // header / response-splitting attacks via the Location header.
  if trimmed
    .bytes()
    .any(|b| b == b'\r' || b == b'\n' || b == 0 || b == b' ' || b == b'\t')
  {
    return None;
  }
  // Use `http::uri::Authority` to enforce RFC 3986 authority syntax. This
  // rejects scheme prefixes, paths, and other smuggled values.
  let _authority: http::uri::Authority = trimmed.parse().ok()?;

  // Strip port if present. IPv6 literals come bracketed (`[::1]:8080`).
  let host = if let Some(after_bracket) = trimmed.strip_prefix('[') {
    let end = after_bracket.find(']')?;
    let bracketed = &trimmed[..=end + 1];
    bracketed.to_string()
  } else {
    trimmed
      .split(':')
      .next()
      .filter(|s| !s.is_empty())?
      .to_string()
  };

  Some(host)
}

/// Builds a router whose fallback redirects every request to the `https://`
/// equivalent on the same host, suitable for binding to port 80 alongside the
/// real TLS listener on `https_port`.
///
/// The `Host` header is parsed as an RFC 3986 authority before being placed
/// in the `Location`; malformed values (including CRLF or scheme smuggling)
/// produce a `400 Bad Request` to prevent open-redirect / cache-poisoning
/// phishing. For strict deployments, prefer
/// [`http_to_https_router_with_allowed_hosts`].
///
/// # Examples
///
/// ```rust,no_run
/// use tako::redirect::http_to_https_router;
///
/// // serve(http80_listener, http_to_https_router(443)).await;
/// ```
pub fn http_to_https_router(https_port: u16) -> crate::router::Router {
  http_to_https_router_inner(https_port, Vec::new())
}

/// Like [`http_to_https_router`] but additionally enforces that the parsed
/// host is one of `allowed_hosts` (case-insensitive). Requests for any other
/// host receive `421 Misdirected Request`. Use this when binding the HTTP
/// listener on the public internet — it blocks the cache-poisoning
/// phishing vector where an attacker sends `Host: evil.com` and the response
/// gets cached against the URL key.
pub fn http_to_https_router_with_allowed_hosts(
  https_port: u16,
  allowed_hosts: impl IntoIterator<Item = impl Into<String>>,
) -> crate::router::Router {
  let allowed: Vec<String> = allowed_hosts
    .into_iter()
    .map(|s| s.into().to_ascii_lowercase())
    .collect();
  http_to_https_router_inner(https_port, allowed)
}

fn http_to_https_router_inner(
  https_port: u16,
  allowed_hosts: Vec<String>,
) -> crate::router::Router {
  let allowed = std::sync::Arc::new(allowed_hosts);
  let mut router = crate::router::Router::new();
  router.fallback(move |req: crate::types::Request| {
    let port = https_port;
    let allowed = allowed.clone();
    async move {
      let host_header = req
        .headers()
        .get(http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
      let Some(host) = validate_host(host_header) else {
        return http::Response::builder()
          .status(StatusCode::BAD_REQUEST)
          .body(TakoBody::from("invalid Host header"))
          .expect("static 400 response is well-formed");
      };
      if !allowed.is_empty() && !allowed.contains(&host.to_ascii_lowercase()) {
        return http::Response::builder()
          .status(StatusCode::MISDIRECTED_REQUEST)
          .body(TakoBody::from("host not allowed"))
          .expect("static 421 response is well-formed");
      }
      let path_and_query = req
        .uri()
        .path_and_query()
        .map_or("/", http::uri::PathAndQuery::as_str);
      let location = if port == 443 {
        format!("https://{host}{path_and_query}")
      } else {
        format!("https://{host}:{port}{path_and_query}")
      };
      Redirect::permanent(location).into_response()
    }
  });
  router
}
