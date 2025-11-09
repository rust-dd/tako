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

use http::header::LOCATION;
use hyper::StatusCode;

use crate::{body::TakoBody, responder::Responder, types::Response};

/// A redirect response builder that implements `Responder`.
pub struct Redirect {
    status: StatusCode,
    location: String,
}

impl Redirect {
    /// Create a redirect with a custom status code.
    pub fn with_status(location: impl Into<String>, status: StatusCode) -> Self {
        Self {
            status,
            location: location.into(),
        }
    }

    /// 302 Found (common temporary redirect).
    pub fn found(location: impl Into<String>) -> Self {
        Self::with_status(location, StatusCode::FOUND)
    }

    /// 303 See Other (commonly used after POST to redirect to a GET page).
    pub fn see_other(location: impl Into<String>) -> Self {
        Self::with_status(location, StatusCode::SEE_OTHER)
    }

    /// 307 Temporary Redirect (preserves the HTTP method).
    pub fn temporary(location: impl Into<String>) -> Self {
        Self::with_status(location, StatusCode::TEMPORARY_REDIRECT)
    }

    /// 301 Moved Permanently.
    pub fn permanent_moved(location: impl Into<String>) -> Self {
        Self::with_status(location, StatusCode::MOVED_PERMANENTLY)
    }

    /// 308 Permanent Redirect.
    pub fn permanent(location: impl Into<String>) -> Self {
        Self::with_status(location, StatusCode::PERMANENT_REDIRECT)
    }
}

impl Responder for Redirect {
    fn into_response(self) -> Response {
        hyper::Response::builder()
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
