//! Bearer token authentication middleware for API security and access control.
//!
//! This module provides middleware for implementing Bearer token authentication as defined
//! in RFC 6750. It supports both static token validation and dynamic verification functions,
//! enabling flexible authentication strategies for APIs. The middleware validates tokens
//! from the Authorization header with a static token set or a boolean verification callback.
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
use subtle::Choice;
use subtle::ConstantTimeEq;
use tako_rs_core::body::TakoBody;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

/// Constant-time match against a list of candidate tokens. See `api_key_auth` for rationale.
fn constant_time_contains(input: &[u8], candidates: &[Vec<u8>]) -> bool {
  let mut found = Choice::from(0u8);
  for candidate in candidates {
    found |= input.ct_eq(candidate.as_slice());
  }
  bool::from(found)
}

/// Custom verification closure for [`BearerAuth`].
pub type BearerAuthVerifyFn = Box<dyn Fn(&str) -> bool + Send + Sync + 'static>;

/// Bearer token authentication with static credentials or a boolean verifier.
///
/// ```rust
/// use tako::middleware::bearer_auth::BearerAuth;
/// let auth = BearerAuth::with_verify(|token| token == "valid-token");
/// ```
pub struct BearerAuth {
  /// Static tokens (raw bytes, scanned in constant time).
  tokens: Option<Vec<Vec<u8>>>,
  /// Custom verification function for dynamic token validation.
  verify: Option<BearerAuthVerifyFn>,
}

/// Implementation of the `BearerAuth` struct, providing methods to configure
/// static tokens, custom verification functions, or a combination of both.
impl BearerAuth {
  /// Creates authentication middleware with a single static token.
  pub fn static_token(token: impl Into<String>) -> Self {
    let token: String = token.into();
    Self {
      tokens: Some(vec![token.into_bytes()]),
      verify: None,
    }
  }

  /// Creates authentication middleware with multiple static tokens.
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

  /// Creates authentication middleware with a custom verification function.
  pub fn with_verify<F>(f: F) -> Self
  where
    F: Fn(&str) -> bool + Clone + Send + Sync + 'static,
  {
    Self {
      tokens: None,
      verify: Some(Box::new(f)),
    }
  }

  /// Creates authentication middleware with both static tokens and custom verification.
  pub fn static_tokens_with_verify<I, F>(tokens: I, f: F) -> Self
  where
    I: IntoIterator,
    I::Item: Into<String>,
    F: Fn(&str) -> bool + Clone + Send + Sync + 'static,
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
  /// Converts the authentication configuration into middleware.
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
        // Extract Bearer token from Authorization header. RFC 7235 §2.1
        // makes the auth-scheme token case-insensitive ("Bearer" / "bearer"
        // / "BEARER" are equivalent), so the prefix match must follow.
        let tok = req
          .headers()
          .get(header::AUTHORIZATION)
          .and_then(|h| h.to_str().ok())
          .and_then(|h| {
            let (scheme, rest) = h.trim_start().split_once(' ')?;
            scheme.eq_ignore_ascii_case("Bearer").then(|| rest.trim())
          });

        // Validate extracted token
        match tok {
          None => {
            // PMW-07: RFC 6750 §3 + RFC 7235 § 3.1 require 401 with a
            // `WWW-Authenticate: Bearer ...` challenge for a missing or
            // malformed Authorization header — 400 prevented the client
            // from re-attempting with credentials. Mirrors `api_key_auth`.
            return http::Response::builder()
              .status(StatusCode::UNAUTHORIZED)
              .header(header::WWW_AUTHENTICATE, bearer_authenticate.clone())
              .body(TakoBody::from("Token is missing"))
              .unwrap()
              .into_response();
          }
          Some(t) => {
            // Check static tokens (constant-time scan)
            if let Some(set) = &tokens
              && constant_time_contains(t.as_bytes(), set)
            {
              return next.run(req).await.into_response();
            }
            // Use custom verification function if available
            if let Some(v) = verify.as_ref()
              && v(t)
            {
              return next.run(req).await.into_response();
            }
          }
        }

        // Return 401 Unauthorized for invalid tokens
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
