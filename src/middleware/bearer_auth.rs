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
pub struct BearerAuth<C, F>
where
    F: Fn(&str) -> Option<C> + Send + Sync + 'static,
    C: Clone + Send + Sync + 'static,
{
    /// Static token set for quick validation.
    tokens: Option<HashSet<String>>,
    /// Custom verification function for dynamic token validation.
    verify: Option<F>,
    /// Phantom data for generic type association.
    _phantom: std::marker::PhantomData<C>,
}

/// Implementation of the `BearerAuth` struct, providing methods to configure
/// static tokens, custom verification functions, or a combination of both.
impl<C, F> BearerAuth<C, F>
where
    F: Fn(&str) -> Option<C> + Clone + Send + Sync + 'static,
    C: Clone + Send + Sync + 'static,
{
    /// Creates authentication middleware with a single static token.
    ///
    /// This is the simplest way to set up Bearer token authentication for applications
    /// that use a single API key or access token. The token is stored in memory and
    /// checked against incoming Authorization headers.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::middleware::bearer_auth::BearerAuth;
    ///
    /// let auth = BearerAuth::<(), _>::static_token("my-secret-api-key");
    /// // Requests with "Authorization: Bearer my-secret-api-key" will be authenticated
    /// ```
    pub fn static_token(token: impl Into<String>) -> Self {
        Self {
            tokens: Some([token.into()].into()),
            verify: None,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Creates authentication middleware with multiple static tokens.
    ///
    /// Allows multiple valid tokens for authentication, useful for supporting
    /// multiple API keys, different service accounts, or temporary tokens alongside
    /// permanent ones. Any token in the collection will authenticate the request.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::middleware::bearer_auth::BearerAuth;
    ///
    /// let auth = BearerAuth::<(), _>::static_tokens([
    ///     "api-key-development",
    ///     "api-key-production",
    ///     "admin-override-token",
    ///     "service-account-token",
    /// ]);
    ///
    /// // Any of the four tokens will authenticate successfully
    /// ```
    pub fn static_tokens<I>(tokens: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        Self {
            tokens: Some(tokens.into_iter().map(Into::into).collect()),
            verify: None,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Creates authentication middleware with a custom verification function.
    ///
    /// The verification function receives the bearer token from the request and can
    /// perform any authentication logic including JWT validation, database lookups,
    /// external API calls, or other verification methods. Returning `Some(claims)`
    /// grants access and injects the claims object into request extensions.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::middleware::bearer_auth::BearerAuth;
    ///
    /// #[derive(Clone)]
    /// struct ApiClaims {
    ///     client_id: String,
    ///     scopes: Vec<String>,
    ///     expires_at: u64,
    /// }
    ///
    /// let auth = BearerAuth::with_verify(|token| {
    ///     // Custom token validation logic
    ///     if token.starts_with("client_") && token.len() > 20 {
    ///         Some(ApiClaims {
    ///             client_id: "client_123".to_string(),
    ///             scopes: vec!["read".to_string(), "write".to_string()],
    ///             expires_at: 1234567890,
    ///         })
    ///     } else if token == "admin_token" {
    ///         Some(ApiClaims {
    ///             client_id: "admin".to_string(),
    ///             scopes: vec!["admin".to_string()],
    ///             expires_at: 9999999999,
    ///         })
    ///     } else {
    ///         None
    ///     }
    /// });
    /// ```
    pub fn with_verify(f: F) -> Self {
        Self {
            tokens: None,
            verify: Some(f),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Creates authentication middleware with both static tokens and custom verification.
    ///
    /// This configuration first checks static tokens for quick validation, then falls
    /// back to the custom verification function if no static match is found. This is
    /// useful for having some hardcoded service tokens while also supporting dynamic
    /// token validation like JWTs or database-backed tokens.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::middleware::bearer_auth::BearerAuth;
    ///
    /// #[derive(Clone)]
    /// struct TokenInfo { user_id: u32, token_type: String }
    ///
    /// let auth = BearerAuth::static_tokens_with_verify(
    ///     ["static-admin-token", "static-service-token"],
    ///     |token| {
    ///         // Check dynamic tokens after static ones
    ///         if token.starts_with("jwt_") {
    ///             // JWT validation logic here
    ///             Some(TokenInfo {
    ///                 user_id: 456,
    ///                 token_type: "jwt".to_string()
    ///             })
    ///         } else if token.starts_with("temp_") {
    ///             // Temporary token validation
    ///             Some(TokenInfo {
    ///                 user_id: 789,
    ///                 token_type: "temporary".to_string()
    ///             })
    ///         } else {
    ///             None
    ///         }
    ///     }
    /// );
    /// ```
    pub fn static_tokens_with_verify<I>(tokens: I, f: F) -> Self
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        Self {
            tokens: Some(tokens.into_iter().map(Into::into).collect()),
            verify: Some(f),
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<C, F> IntoMiddleware for BearerAuth<C, F>
where
    F: Fn(&str) -> Option<C> + Send + Sync + 'static,
    C: Clone + Send + Sync + 'static,
{
    /// Converts the authentication configuration into middleware.
    ///
    /// The resulting middleware validates Bearer tokens from the Authorization header.
    /// On successful authentication, the request proceeds to the next middleware.
    /// On failure, returns a 401 Unauthorized or 400 Bad Request response with
    /// appropriate WWW-Authenticate header.
    ///
    /// If a verification function returns claims or user data, it is inserted into
    /// the request extensions for access by downstream handlers.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::middleware::bearer_auth::BearerAuth;
    /// use tako::middleware::IntoMiddleware;
    ///
    /// let auth_middleware = BearerAuth::<(), _>::static_tokens([
    ///     "development-key",
    ///     "production-key",
    /// ]).into_middleware();
    ///
    /// // Use in router:
    /// // router.middleware(auth_middleware);
    /// ```
    fn into_middleware(
        self,
    ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
    + Clone
    + Send
    + Sync
    + 'static {
        let tokens = self.tokens.map(Arc::new);
        let verify = self.verify.map(Arc::new);

        move |mut req: Request, next: Next| {
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
                        return hyper::Response::builder()
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
                            if let Some(claims) = v(t) {
                                req.extensions_mut().insert(claims);
                                return next.run(req).await.into_response();
                            }
                        }
                    }
                }

                // Return 401 Unauthorized for invalid tokens
                hyper::Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header(header::WWW_AUTHENTICATE, "Bearer")
                    .body(TakoBody::empty())
                    .unwrap()
                    .into_response()
            })
        }
    }
}
