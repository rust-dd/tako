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
  fn into_response(self) -> Response {
    http::Response::builder()
      .status(self.status)
      .header(LOCATION, self.location)
      .body(TakoBody::empty())
      .unwrap()
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

/// Builds a router whose fallback redirects every request to the `https://`
/// equivalent on the same host, suitable for binding to port 80 alongside the
/// real TLS listener on `https_port`.
///
/// # Examples
///
/// ```rust,no_run
/// use tako::redirect::http_to_https_router;
///
/// // serve(http80_listener, http_to_https_router(443)).await;
/// ```
pub fn http_to_https_router(https_port: u16) -> crate::router::Router {
  let mut router = crate::router::Router::new();
  router.fallback(move |req: crate::types::Request| {
    let port = https_port;
    async move {
      let host_header = req
        .headers()
        .get(http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
      let host = host_header.split(':').next().unwrap_or("").trim();
      let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
      let location = if port == 443 {
        format!("https://{host}{path_and_query}")
      } else {
        format!("https://{host}:{port}{path_and_query}")
      };
      Redirect::permanent(location)
    }
  });
  router
}
