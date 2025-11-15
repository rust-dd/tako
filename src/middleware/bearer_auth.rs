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

use bytes::Bytes;
use http::{StatusCode, header};
use http_body_util::Full;
use std::{collections::HashSet, future::Future, pin::Pin, sync::Arc};

use crate::{
  body::TakoBody,
  middleware::{IntoMiddleware, Next},
  responder::Responder,
  types::{Request, Response},
};

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
  /// Static token set for quick validation.
  tokens: Option<HashSet<String>>,
  /// Custom verification function for dynamic token validation.
  verify: Option<Box<dyn Fn(&str) -> bool + Send + Sync + 'static>>,
}

/// Implementation of the `BearerAuth` struct, providing methods to configure
/// static tokens, custom verification functions, or a combination of both.
impl BearerAuth {
  /// Creates authentication middleware with a single static token.
  pub fn static_token(token: impl Into<String>) -> Self {
    Self {
      tokens: Some([token.into()].into()),
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
      tokens: Some(tokens.into_iter().map(Into::into).collect()),
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
      tokens: Some(tokens.into_iter().map(Into::into).collect()),
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

    move |req: Request, next: Next| {
      let tokens = tokens.clone();
      let verify = verify.clone();

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
              .header(header::WWW_AUTHENTICATE, "Bearer")
              .body(TakoBody::new(Full::from(Bytes::from("Token is missing"))))
              .unwrap()
              .into_response();
          }
          Some(t) => {
            // Check static token set first
            if let Some(set) = &tokens {
              if set.contains(t) {
                return next.run(req).await.into_response();
              }
            }
            // Use custom verification function if available
            if let Some(v) = verify.as_ref() {
              if v(t) {
                return next.run(req).await.into_response();
              }
            }
          }
        }

        // Return 401 Unauthorized for invalid tokens
        http::Response::builder()
          .status(StatusCode::UNAUTHORIZED)
          .header(header::WWW_AUTHENTICATE, "Bearer")
          .body(TakoBody::empty())
          .unwrap()
          .into_response()
      })
    }
  }
}
