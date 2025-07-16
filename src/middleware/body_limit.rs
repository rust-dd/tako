//! Request body size limiting middleware for preventing resource exhaustion attacks.
//!
//! This module provides middleware for limiting the size of HTTP request bodies to prevent
//! denial-of-service attacks and resource exhaustion. It supports both static size limits
//! and dynamic limits based on request properties. The middleware performs fast rejection
//! using the Content-Length header when available, avoiding unnecessary body processing
//! for oversized requests.
//!
//! # Examples
//!
//! ```rust
//! use tako::middleware::body_limit::BodyLimit;
//! use tako::middleware::IntoMiddleware;
//!
//! // Static 1MB limit for all requests
//! let limit = BodyLimit::new(1024 * 1024);
//! let middleware = limit.into_middleware();
//!
//! // Dynamic limit based on request properties
//! let dynamic = BodyLimit::with_dynamic_limit(|req| {
//!     if req.uri().path().starts_with("/upload") {
//!         50 * 1024 * 1024 // 50MB for uploads
//!     } else {
//!         1024 * 1024 // 1MB for other requests
//!     }
//! });
//!
//! // Combined static and dynamic limits
//! let combined = BodyLimit::new_with_dynamic(5 * 1024 * 1024, |req| {
//!     // Dynamic limit overrides static one
//!     if req.headers().get("x-large-upload").is_some() {
//!         100 * 1024 * 1024 // 100MB for special uploads
//!     } else {
//!         2 * 1024 * 1024 // 2MB override
//!     }
//! });
//! ```

use std::{future::Future, pin::Pin, sync::Arc};

use http::{StatusCode, header::CONTENT_LENGTH};

use crate::{
    middleware::{IntoMiddleware, Next},
    responder::Responder,
    types::{Request, Response},
};

/// Request body size limiting middleware configuration.
///
/// `BodyLimit` provides configurable middleware for limiting HTTP request body sizes
/// to prevent resource exhaustion and denial-of-service attacks. It supports both
/// static size limits and dynamic limits that can vary based on request properties
/// such as path, headers, or other metadata. The middleware performs efficient
/// early rejection using Content-Length headers when available.
///
/// # Type Parameters
///
/// * `F` - Dynamic limit function type that takes a request and returns the size limit
///
/// # Examples
///
/// ```rust
/// use tako::middleware::body_limit::BodyLimit;
/// use tako::types::Request;
///
/// // Simple static limit
/// let static_limit = BodyLimit::new(1024 * 1024); // 1MB
///
/// // Dynamic limit based on endpoint
/// let dynamic_limit = BodyLimit::with_dynamic_limit(|req| {
///     match req.uri().path() {
///         "/api/upload" => 10 * 1024 * 1024, // 10MB for uploads
///         "/api/data" => 5 * 1024 * 1024,    // 5MB for data
///         _ => 1024 * 1024,                  // 1MB default
///     }
/// });
/// ```
pub struct BodyLimit<F>
where
    F: Fn(&Request) -> usize + Send + Sync + 'static,
{
    /// Static size limit in bytes, if configured.
    limit: Option<usize>,
    /// Dynamic limit function for request-based limits.
    dynamic_limit: Option<F>,
}

impl<F> BodyLimit<F>
where
    F: Fn(&Request) -> usize + Send + Sync + 'static,
{
    /// Creates a body limit middleware with a fixed size limit.
    ///
    /// The limit applies to all requests processed by this middleware. Requests
    /// with Content-Length headers exceeding this limit are rejected immediately
    /// without reading the body.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::middleware::body_limit::BodyLimit;
    ///
    /// // 5MB limit for all requests
    /// let limit = BodyLimit::new(5 * 1024 * 1024);
    ///
    /// // 100KB limit for small API requests
    /// let small_limit = BodyLimit::new(100 * 1024);
    /// ```
    pub fn new(limit: usize) -> Self {
        Self {
            limit: Some(limit),
            dynamic_limit: None,
        }
    }

    /// Creates a body limit middleware with a dynamic limit function.
    ///
    /// The provided function is called for each request to determine the appropriate
    /// size limit. This allows different limits based on request properties such as
    /// path, headers, authentication status, or other metadata.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::middleware::body_limit::BodyLimit;
    ///
    /// let dynamic = BodyLimit::with_dynamic_limit(|req| {
    ///     // Different limits based on user type
    ///     if req.headers().get("x-user-type") == Some(&"premium".parse().unwrap()) {
    ///         50 * 1024 * 1024 // 50MB for premium users
    ///     } else {
    ///         1024 * 1024 // 1MB for regular users
    ///     }
    /// });
    ///
    /// // Path-based limits
    /// let path_based = BodyLimit::with_dynamic_limit(|req| {
    ///     match req.uri().path() {
    ///         p if p.starts_with("/api/v1/") => 2 * 1024 * 1024,
    ///         p if p.starts_with("/upload/") => 100 * 1024 * 1024,
    ///         _ => 512 * 1024,
    ///     }
    /// });
    /// ```
    pub fn with_dynamic_limit(f: F) -> Self {
        Self {
            limit: None,
            dynamic_limit: Some(f),
        }
    }

    /// Creates a body limit middleware with both static and dynamic limits.
    ///
    /// The dynamic function's return value takes precedence over the static limit.
    /// This is useful for having a default limit while allowing specific requests
    /// to override it based on their properties.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::middleware::body_limit::BodyLimit;
    ///
    /// let combined = BodyLimit::new_with_dynamic(
    ///     1024 * 1024, // 1MB default
    ///     |req| {
    ///         // Override for specific cases
    ///         if req.uri().path() == "/admin/bulk-import" {
    ///             500 * 1024 * 1024 // 500MB for bulk imports
    ///         } else if req.headers().get("content-type")
    ///             .and_then(|v| v.to_str().ok())
    ///             .map(|s| s.starts_with("image/"))
    ///             .unwrap_or(false) {
    ///             10 * 1024 * 1024 // 10MB for images
    ///         } else {
    ///             2 * 1024 * 1024 // 2MB override for others
    ///         }
    ///     }
    /// );
    /// ```
    pub fn new_with_dynamic(limit: usize, f: F) -> Self {
        Self {
            limit: Some(limit),
            dynamic_limit: Some(f),
        }
    }
}

impl<F> IntoMiddleware for BodyLimit<F>
where
    F: Fn(&Request) -> usize + Send + Sync + 'static,
{
    /// Converts the body limit configuration into middleware.
    ///
    /// The resulting middleware checks request body sizes against configured limits.
    /// It performs fast rejection using Content-Length headers when available, avoiding
    /// the need to read oversized request bodies. If no Content-Length header is present,
    /// the request proceeds (though runtime truncation could be added in the future).
    ///
    /// The effective limit is determined in order of precedence:
    /// 1. Dynamic function result (if configured)
    /// 2. Static limit (if configured)
    /// 3. Default limit of 10 MiB
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::middleware::body_limit::BodyLimit;
    /// use tako::middleware::IntoMiddleware;
    ///
    /// let limit_middleware = BodyLimit::new(2 * 1024 * 1024) // 2MB
    ///     .into_middleware();
    ///
    /// // Use in router:
    /// // router.middleware(limit_middleware);
    /// ```
    fn into_middleware(
        self,
    ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
    + Clone
    + Send
    + Sync
    + 'static {
        let static_limit = self.limit;
        let dynamic_limit = self.dynamic_limit.map(Arc::new);

        move |req: Request, next: Next| {
            let dynamic_limit = dynamic_limit.clone();

            Box::pin(async move {
                // Determine effective limit: dynamic → static → default 10 MiB
                let limit = dynamic_limit
                    .as_ref()
                    .map(|f| f(&req))
                    .or(static_limit)
                    .unwrap_or(10 * 1024 * 1024);

                // Fast-path rejection via Content-Length header
                if let Some(len) = req
                    .headers()
                    .get(CONTENT_LENGTH)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    if len > limit {
                        return (StatusCode::PAYLOAD_TOO_LARGE, "Body exceeds allowed size")
                            .into_response();
                    }
                }

                // TODO: add run-time stream truncation if your Body supports it.
                next.run(req).await.into_response()
            })
        }
    }
}
