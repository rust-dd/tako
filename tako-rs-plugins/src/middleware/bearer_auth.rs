//! RFC 6750 Bearer authentication: a token passes when it matches a static token (compared
//! in constant time) or the verify callback accepts it.
//!
//! # Examples
//!
//! ```rust
//! use tako::middleware::bearer_auth::BearerAuth;
//! use tako::middleware::IntoMiddleware;
//!
//! // Single static token.
//! let auth = BearerAuth::static_token("secret-api-key");
//! let mw = auth.into_middleware();
//!
//! // Multiple valid tokens.
//! let multi = BearerAuth::static_tokens([
//!     "token1",
//!     "token2",
//!     "admin-token",
//! ]);
//!
//! // Dynamic verify callback — returns `bool`, not a claims object.
//! let dynamic = BearerAuth::with_verify(|token| token.starts_with("user_"));
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::HeaderValue;
use http::StatusCode;
use http::header;
use tako_rs_core::body::TakoBody;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

use super::api_key_auth::constant_time_contains;

/// Custom verification closure for [`BearerAuth`].
pub type BearerAuthVerifyFn = Box<dyn Fn(&str) -> bool + Send + Sync + 'static>;

/// Rejects a missing or unknown token with 401 and a `WWW-Authenticate: Bearer` challenge.
///
/// ```rust
/// use tako::middleware::bearer_auth::BearerAuth;
/// let auth = BearerAuth::with_verify(|token| token == "valid-token");
/// ```
pub struct BearerAuth {
  tokens: Option<Vec<Vec<u8>>>,
  verify: Option<BearerAuthVerifyFn>,
}

impl BearerAuth {
  pub fn static_token(token: impl Into<String>) -> Self {
    let token: String = token.into();
    Self {
      tokens: Some(vec![token.into_bytes()]),
      verify: None,
    }
  }

  pub fn static_tokens<I>(tokens: I) -> Self
  where
    I: IntoIterator,
    I::Item: Into<String>,
  {
    Self {
      tokens: Some(
        tokens
          .into_iter()
          .map(|t| Into::<String>::into(t).into_bytes())
          .collect(),
      ),
      verify: None,
    }
  }

  pub fn with_verify<F>(f: F) -> Self
  where
    F: Fn(&str) -> bool + Send + Sync + 'static,
  {
    Self {
      tokens: None,
      verify: Some(Box::new(f)),
    }
  }

  pub fn static_tokens_with_verify<I, F>(tokens: I, f: F) -> Self
  where
    I: IntoIterator,
    I::Item: Into<String>,
    F: Fn(&str) -> bool + Send + Sync + 'static,
  {
    Self {
      tokens: Some(
        tokens
          .into_iter()
          .map(|t| Into::<String>::into(t).into_bytes())
          .collect(),
      ),
      verify: Some(Box::new(f)),
    }
  }
}

impl IntoMiddleware for BearerAuth {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let tokens = self.tokens.map(Arc::new);
    let verify = self.verify.map(Arc::new);
    let bearer_authenticate = HeaderValue::from_static("Bearer");

    move |req: Request, next: Next| {
      let tokens = tokens.clone();
      let verify = verify.clone();
      let bearer_authenticate = bearer_authenticate.clone();

      Box::pin(async move {
        // RFC 7235 §2.1: the auth-scheme token is case-insensitive.
        let tok = req
          .headers()
          .get(header::AUTHORIZATION)
          .and_then(|h| h.to_str().ok())
          .and_then(|h| {
            let (scheme, rest) = h.trim_start().split_once(' ')?;
            scheme.eq_ignore_ascii_case("Bearer").then(|| rest.trim())
          });

        match tok {
          None => {
            // RFC 6750 §3: a missing credential gets a 401 challenge so the client can retry.
            return http::Response::builder()
              .status(StatusCode::UNAUTHORIZED)
              .header(header::WWW_AUTHENTICATE, bearer_authenticate.clone())
              .body(TakoBody::from("Token is missing"))
              .unwrap()
              .into_response();
          }
          Some(t) => {
            if let Some(set) = &tokens
              && constant_time_contains(t.as_bytes(), set)
            {
              return next.run(req).await.into_response();
            }
            if let Some(v) = verify.as_ref()
              && v(t)
            {
              return next.run(req).await.into_response();
            }
          }
        }

        http::Response::builder()
          .status(StatusCode::UNAUTHORIZED)
          .header(header::WWW_AUTHENTICATE, bearer_authenticate)
          .body(TakoBody::empty())
          .unwrap()
          .into_response()
      })
    }
  }
}
