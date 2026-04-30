//! Bearer token authentication middleware for API security and access control.
//!
//! This module provides middleware for implementing Bearer token authentication as defined
//! in RFC 6750. It supports both static token validation and dynamic verification functions,
//! enabling flexible authentication strategies for APIs. The middleware validates tokens
//! from the Authorization header and can inject custom claims or user objects into request
//! extensions for downstream handlers.
//!
//! # Examples
//!
//! ```rust
//! use tako::middleware::bearer_auth::BearerAuth;
//! use tako::middleware::IntoMiddleware;
//!
//! // Single static token
//! let auth = BearerAuth::<(), _>::static_token("secret-api-key");
//! let middleware = auth.into_middleware();
//!
//! // Multiple valid tokens
//! let multi_auth = BearerAuth::<(), _>::static_tokens([
//!     "token1",
//!     "token2",
//!     "admin-token",
//! ]);
//!
//! // Dynamic verification with claims
//! #[derive(Clone)]
//! struct Claims { user_id: u32, role: String }
//!
//! let dynamic_auth = BearerAuth::with_verify(|token| {
//!     if token.starts_with("user_") {
//!         Some(Claims { user_id: 123, role: "user".to_string() })
//!     } else {
//!         None
//!     }
//! });
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::HeaderValue;
use http::StatusCode;
use http::header;
use subtle::Choice;
use subtle::ConstantTimeEq;
use tako_core::body::TakoBody;
use tako_core::middleware::IntoMiddleware;
use tako_core::middleware::Next;
use tako_core::responder::Responder;
use tako_core::types::Request;
use tako_core::types::Response;

/// Constant-time match against a list of candidate tokens. See `api_key_auth` for rationale.
fn constant_time_contains(input: &[u8], candidates: &[Vec<u8>]) -> bool {
  let mut found = Choice::from(0u8);
  for candidate in candidates {
    found |= input.ct_eq(candidate.as_slice());
  }
  bool::from(found)
}

/// Bearer token authentication middleware configuration.
///
/// `BearerAuth` provides flexible configuration for Bearer token authentication using either
/// static token validation, dynamic verification functions, or both. The middleware validates
/// tokens from the Authorization header and can inject custom claims or user objects into
/// request extensions for use by downstream handlers.
///
/// # Type Parameters
///
/// * `C` - Claims or user object type returned by verification functions
/// * `F` - Verification function type that takes a token and returns `Option<C>`
///
/// # Examples
///
/// ```rust
/// use tako::middleware::bearer_auth::BearerAuth;
/// use std::collections::HashSet;
///
/// // Simple static token validation
/// let auth = BearerAuth::<(), _>::static_token("api-key-12345");
///
/// // Multiple valid tokens
/// let multi = BearerAuth::<(), _>::static_tokens([
///     "development-key",
///     "staging-key",
///     "admin-key",
/// ]);
///
/// // Custom verification with user claims
/// #[derive(Clone)]
/// struct UserClaims { id: u32, permissions: Vec<String> }
///
/// let custom = BearerAuth::with_verify(|token| {
///     // Verify JWT, API key lookup, etc.
///     if token == "valid-jwt-token" {
///         Some(UserClaims {
///             id: 42,
///             permissions: vec!["read".to_string(), "write".to_string()],
///         })
///     } else {
///         None
///     }
/// });
/// ```
pub struct BearerAuth {
  /// Static tokens (raw bytes, scanned in constant time).
  tokens: Option<Vec<Vec<u8>>>,
  /// Custom verification function for dynamic token validation.
  verify: Option<Box<dyn Fn(&str) -> bool + Send + Sync + 'static>>,
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
        // Extract Bearer token from Authorization header
        let tok = req
          .headers()
          .get(header::AUTHORIZATION)
          .and_then(|h| h.to_str().ok())
          .and_then(|h| h.strip_prefix("Bearer "))
          .map(str::trim);

        // Validate extracted token
        match tok {
          None => {
            return http::Response::builder()
              .status(StatusCode::BAD_REQUEST)
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
