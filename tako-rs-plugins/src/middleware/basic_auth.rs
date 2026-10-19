//! RFC 7617 Basic authentication against static credentials (compared in constant time)
//! and/or a verify callback.
//!
//! # Examples
//!
//! ```rust
//! use tako::middleware::basic_auth::BasicAuth;
//! use tako::middleware::IntoMiddleware;
//!
//! // Single user.
//! let auth = BasicAuth::single("admin", "password");
//! let mw = auth.into_middleware();
//!
//! // Multiple users with custom realm.
//! let multi = BasicAuth::multiple([
//!     ("alice", "secret1"),
//!     ("bob", "secret2"),
//! ]).realm("Admin Area");
//!
//! // Dynamic verify callback — returns `bool`, not a user object.
//! let dynamic = BasicAuth::with_verify(|username, password| {
//!     username == "user" && password == "pass"
//! });
//! ```

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use base64::Engine;
use http::HeaderValue;
use http::StatusCode;
use http::header;
use subtle::Choice;
use subtle::ConstantTimeEq;
use tako_rs_core::body::TakoBody;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::BuildHasher;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

/// Custom verification closure for [`BasicAuth`].
pub type BasicAuthVerifyFn = Arc<dyn Fn(&str, &str) -> bool + Send + Sync + 'static>;

/// Credentials pass when a static pair matches or the verify callback accepts them.
///
/// ```rust
/// use tako::middleware::basic_auth::BasicAuth;
/// let auth = BasicAuth::with_verify(|user, password| user == "admin" && password == "secret");
/// ```
pub struct BasicAuth {
  users: Option<Arc<HashMap<String, String, BuildHasher>>>,
  verify: Option<BasicAuthVerifyFn>,
  realm: &'static str,
}

impl BasicAuth {
  pub fn single(user: impl Into<String>, pass: impl Into<String>) -> Self {
    Self::multiple(std::iter::once((user, pass)))
  }

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

  /// Defaults to `Restricted`; a realm that is not a valid header value is omitted.
  pub fn realm(mut self, r: &'static str) -> Self {
    self.realm = r;
    self
  }
}

impl IntoMiddleware for BasicAuth {
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
    // RFC 7617 §2.1 makes the realm optional, so an unencodable one falls back to `Basic`.
    let www_authenticate = HeaderValue::from_str(&format!("Basic realm=\"{realm}\""))
      .unwrap_or_else(|_| HeaderValue::from_static("Basic"));

    move |req: Request, next: Next| {
      let users = users.clone();
      let verify = verify.clone();
      let www_authenticate = www_authenticate.clone();

      Box::pin(async move {
        // RFC 7235 §2.1: the auth-scheme token is case-insensitive.
        let creds = req
          .headers()
          .get(header::AUTHORIZATION)
          .and_then(|h| h.to_str().ok())
          .and_then(|h| {
            let (scheme, rest) = h.trim_start().split_once(' ')?;
            scheme.eq_ignore_ascii_case("Basic").then(|| rest.trim())
          })
          .and_then(|b64| base64::engine::general_purpose::STANDARD.decode(b64).ok());

        match creds {
          Some(raw) => {
            let Some(decoded) = std::str::from_utf8(&raw).ok() else {
              let mut res = Response::new(TakoBody::empty());
              *res.status_mut() = StatusCode::UNAUTHORIZED;
              res
                .headers_mut()
                .append(header::WWW_AUTHENTICATE, www_authenticate.clone());
              return res;
            };
            let Some((u, p)) = decoded.split_once(':') else {
              let mut res = Response::new(TakoBody::empty());
              *res.status_mut() = StatusCode::UNAUTHORIZED;
              res
                .headers_mut()
                .append(header::WWW_AUTHENTICATE, www_authenticate.clone());
              return res;
            };

            // Scan every entry and compare both fields so the time to 401 leaks neither
            // whether the user exists nor a matching password prefix.
            let mut authed = Choice::from(0u8);
            if let Some(map) = users.as_ref() {
              for (known_user, known_pw) in map.iter() {
                authed |= known_user.as_bytes().ct_eq(u.as_bytes())
                  & known_pw.as_bytes().ct_eq(p.as_bytes());
              }
            }
            if bool::from(authed) {
              return next.run(req).await.into_response();
            }

            if let Some(cb) = &verify
              && cb(u, p)
            {
              return next.run(req).await.into_response();
            }
          }
          None => {
            return http::Response::builder()
              .status(StatusCode::UNAUTHORIZED)
              .header(header::WWW_AUTHENTICATE, www_authenticate.clone())
              .body(TakoBody::from("Missing credentials"))
              .unwrap()
              .into_response();
          }
        }

        let mut res = Response::new(TakoBody::empty());
        *res.status_mut() = StatusCode::UNAUTHORIZED;
        res
          .headers_mut()
          .append(header::WWW_AUTHENTICATE, www_authenticate);
        res
      })
    }
  }
}
