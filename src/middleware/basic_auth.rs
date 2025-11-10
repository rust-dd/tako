//! Basic HTTP authentication middleware for securing web application endpoints.
//!
//! This module provides middleware for implementing RFC 7617 Basic HTTP Authentication.
//! It supports both static user credentials and dynamic verification functions, allowing
//! flexible authentication strategies. The middleware validates credentials from the
//! Authorization header and can inject user objects into request extensions for use
//! by downstream handlers.
//!
//! # Examples
//!
//! ```rust
//! use tako::middleware::basic_auth::BasicAuth;
//! use tako::middleware::IntoMiddleware;
//!
//! // Single user authentication
//! let auth = BasicAuth::<(), _>::single("admin", "password");
//! let middleware = auth.into_middleware();
//!
//! // Multiple users with custom realm
//! let multi_auth = BasicAuth::<(), _>::multiple([
//!     ("alice", "secret1"),
//!     ("bob", "secret2"),
//! ]).realm("Admin Area");
//!
//! // Dynamic verification with user object
//! #[derive(Clone)]
//! struct User { id: u32, name: String }
//!
//! let dynamic_auth = BasicAuth::with_verify(|username, password| {
//!     if username == "user" && password == "pass" {
//!         Some(User { id: 1, name: username.to_string() })
//!     } else {
//!         None
//!     }
//! });
//! ```

use crate::{
  body::TakoBody,
  middleware::{IntoMiddleware, Next},
  responder::Responder,
  types::{Request, Response},
};
use base64::Engine;
use bytes::Bytes;
use http::{HeaderValue, StatusCode, header};
use http_body_util::Full;
use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

/// Basic HTTP authentication middleware configuration.
///
/// `BasicAuth` provides flexible configuration for HTTP Basic authentication using either
/// static user credentials, dynamic verification functions, or both. The middleware
/// validates credentials from the Authorization header and can inject authenticated
/// user objects into request extensions for downstream handlers.
///
/// # Type Parameters
///
/// * `U` - User object type returned by verification functions
/// * `F` - Verification function type that takes username/password and returns `Option<U>`
///
/// # Examples
///
/// ```rust
/// use tako::middleware::basic_auth::BasicAuth;
/// use std::collections::HashMap;
///
/// // Simple static authentication
/// let auth = BasicAuth::<(), _>::single("admin", "secret");
///
/// // Multiple static users
/// let multi = BasicAuth::<(), _>::multiple([
///     ("user1", "pass1"),
///     ("user2", "pass2"),
/// ]);
///
/// // Custom verification with user data
/// #[derive(Clone)]
/// struct UserInfo { id: u32, role: String }
///
/// let custom = BasicAuth::with_verify(|user, pass| {
///     // Verify against database, LDAP, etc.
///     if user == "admin" && pass == "secret" {
///         Some(UserInfo { id: 1, role: "admin".to_string() })
///     } else {
///         None
///     }
/// });
/// ```
pub struct BasicAuth {
  /// Static user credentials map (username -> password).
  users: Option<Arc<HashMap<String, String>>>,
  /// Custom verification function for dynamic authentication.
  verify: Option<Arc<dyn Fn(&str, &str) -> bool + Send + Sync + 'static>>,
  /// Authentication realm for WWW-Authenticate header.
  realm: &'static str,
}

impl BasicAuth {
  /// Creates authentication middleware with a single static user credential.
  pub fn single(user: impl Into<String>, pass: impl Into<String>) -> Self {
    Self::multiple(std::iter::once((user, pass)))
  }

  /// Creates authentication middleware with multiple static user credentials.
  pub fn multiple<I, T, P>(pairs: I) -> Self
  where
    I: IntoIterator<Item = (T, P)>,
    T: Into<String>,
    P: Into<String>,
  {
    Self {
      users: Some(Arc::new(
        pairs
          .into_iter()
          .map(|(u, p)| (u.into(), p.into()))
          .collect(),
      )),
      verify: None,
      realm: "Restricted",
    }
  }

  /// Creates authentication middleware with a custom verification function.
  pub fn with_verify<F>(cb: F) -> Self
  where
    F: Fn(&str, &str) -> bool + Send + Sync + 'static,
  {
    Self {
      users: None,
      verify: Some(Arc::new(cb)),
      realm: "Restricted",
    }
  }

  /// Creates authentication middleware with both static credentials and custom verification.
  pub fn users_with_verify<I, S, F>(pairs: I, cb: F) -> Self
  where
    I: IntoIterator<Item = (S, S)>,
    S: Into<String>,
    F: Fn(&str, &str) -> bool + Send + Sync + 'static,
  {
    Self {
      users: Some(Arc::new(
        pairs
          .into_iter()
          .map(|(u, p)| (u.into(), p.into()))
          .collect(),
      )),
      verify: Some(Arc::new(cb)),
      realm: "Restricted",
    }
  }

  /// Sets the authentication realm for the WWW-Authenticate header.
  pub fn realm(mut self, r: &'static str) -> Self {
    self.realm = r;
    self
  }
}

impl IntoMiddleware for BasicAuth {
  /// Converts the authentication configuration into middleware.
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let users = self.users;
    let verify = self.verify;
    let realm = self.realm;

    move |req: Request, next: Next| {
      let users = users.clone();
      let verify = verify.clone();

      Box::pin(async move {
        // Extract Basic credentials from Authorization header
        let creds = req
          .headers()
          .get(header::AUTHORIZATION)
          .and_then(|h| h.to_str().ok())
          .and_then(|h| h.strip_prefix("Basic "))
          .and_then(|b64| base64::engine::general_purpose::STANDARD.decode(b64).ok())
          .and_then(|raw| String::from_utf8(raw).ok())
          .and_then(|s| s.split_once(':').map(|(u, p)| (u.to_owned(), p.to_owned())));

        match creds {
          Some((u, p)) => {
            // Check static user credentials first
            if users
              .as_ref()
              .and_then(|map| map.get(&u))
              .map(|pw| pw == &p)
              .unwrap_or(false)
            {
              return next.run(req).await.into_response();
            }

            // Use custom verification function if available
            if let Some(cb) = &verify {
              if cb(&u, &p) {
                return next.run(req).await.into_response();
              }
            }
          }
          None => {
            return hyper::Response::builder()
              .status(StatusCode::UNAUTHORIZED)
              .header(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_str(&format!("Basic realm=\"{realm}\"")).unwrap(),
              )
              .body(TakoBody::new(Full::from(Bytes::from(
                "Missing credentials",
              ))))
              .unwrap()
              .into_response();
          }
        }

        // Return 401 Unauthorized for invalid credentials
        let mut res = Response::new(TakoBody::empty());
        *res.status_mut() = StatusCode::UNAUTHORIZED;
        res.headers_mut().append(
          header::WWW_AUTHENTICATE,
          HeaderValue::from_str(&format!("Basic realm=\"{realm}\"")).unwrap(),
        );
        res
      })
    }
  }
}
