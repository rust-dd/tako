//! API Key authentication middleware for simple token-based access control.
//!
//! This module provides middleware for validating API keys from HTTP headers or query
//! parameters. It supports multiple key sources, custom header names, and dynamic
//! key verification functions for flexible authentication strategies.
//!
//! # Examples
//!
//! ```rust
//! use tako::middleware::api_key_auth::{ApiKeyAuth, ApiKeyLocation};
//! use tako::middleware::IntoMiddleware;
//!
//! // Single API key from header
//! let auth = ApiKeyAuth::new("secret-api-key");
//! let middleware = auth.into_middleware();
//!
//! // Multiple valid keys
//! let multi_auth = ApiKeyAuth::from_keys(["key1", "key2", "admin-key"]);
//!
//! // Custom header name
//! let custom_auth = ApiKeyAuth::new("secret")
//!     .header_name("X-Custom-Key");
//!
//! // From query parameter
//! let query_auth = ApiKeyAuth::new("secret")
//!     .location(ApiKeyLocation::Query("api_key"));
//!
//! // Dynamic verification
//! let dynamic_auth = ApiKeyAuth::with_verify(|key| {
//!     key.starts_with("valid_")
//! });
//! ```

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use http::StatusCode;
use http::header;
use http_body_util::Full;

use crate::body::TakoBody;
use crate::middleware::IntoMiddleware;
use crate::middleware::Next;
use crate::responder::Responder;
use crate::types::BuildHasher;
use crate::types::Request;
use crate::types::Response;

/// Location where the API key should be extracted from.
#[derive(Clone)]
pub enum ApiKeyLocation {
  /// Extract from HTTP header with the given name.
  Header(&'static str),
  /// Extract from query parameter with the given name.
  Query(&'static str),
  /// Try header first, then query parameter.
  HeaderOrQuery(&'static str, &'static str),
}

impl Default for ApiKeyLocation {
  fn default() -> Self {
    Self::Header("X-API-Key")
  }
}

/// API Key authentication middleware configuration.
///
/// `ApiKeyAuth` provides flexible configuration for API key authentication,
/// supporting static keys, dynamic verification, and multiple extraction locations.
///
/// # Examples
///
/// ```rust
/// use tako::middleware::api_key_auth::{ApiKeyAuth, ApiKeyLocation};
///
/// // Simple static key
/// let auth = ApiKeyAuth::new("my-secret-key");
///
/// // Multiple keys with custom location
/// let auth = ApiKeyAuth::from_keys(["key1", "key2"])
///     .location(ApiKeyLocation::Query("apikey"));
///
/// // Dynamic verification
/// let auth = ApiKeyAuth::with_verify(|key| {
///     // Lookup in database, validate format, etc.
///     key.len() == 32 && key.chars().all(|c| c.is_ascii_hexdigit())
/// });
/// ```
pub struct ApiKeyAuth {
  /// Static API key set for quick validation.
  keys: Option<HashSet<String, BuildHasher>>,
  /// Custom verification function for dynamic key validation.
  verify: Option<Arc<dyn Fn(&str) -> bool + Send + Sync + 'static>>,
  /// Location to extract the API key from.
  location: ApiKeyLocation,
}

impl ApiKeyAuth {
  /// Creates authentication middleware with a single static API key.
  ///
  /// By default, the key is extracted from the `X-API-Key` header.
  pub fn new(key: impl Into<String>) -> Self {
    let mut set: HashSet<String, BuildHasher> = HashSet::with_hasher(BuildHasher::default());
    set.insert(key.into());
    Self {
      keys: Some(set),
      verify: None,
      location: ApiKeyLocation::default(),
    }
  }

  /// Creates authentication middleware with multiple static API keys.
  pub fn from_keys<I>(keys: I) -> Self
  where
    I: IntoIterator,
    I::Item: Into<String>,
  {
    Self {
      keys: Some(keys.into_iter().map(Into::into).collect()),
      verify: None,
      location: ApiKeyLocation::default(),
    }
  }

  /// Creates authentication middleware with a custom verification function.
  pub fn with_verify<F>(f: F) -> Self
  where
    F: Fn(&str) -> bool + Send + Sync + 'static,
  {
    Self {
      keys: None,
      verify: Some(Arc::new(f)),
      location: ApiKeyLocation::default(),
    }
  }

  /// Creates authentication with both static keys and custom verification.
  pub fn from_keys_with_verify<I, F>(keys: I, f: F) -> Self
  where
    I: IntoIterator,
    I::Item: Into<String>,
    F: Fn(&str) -> bool + Send + Sync + 'static,
  {
    Self {
      keys: Some(keys.into_iter().map(Into::into).collect()),
      verify: Some(Arc::new(f)),
      location: ApiKeyLocation::default(),
    }
  }

  /// Sets the location where the API key should be extracted from.
  pub fn location(mut self, location: ApiKeyLocation) -> Self {
    self.location = location;
    self
  }

  /// Sets a custom header name for API key extraction.
  ///
  /// This is a convenience method equivalent to
  /// `.location(ApiKeyLocation::Header(name))`.
  pub fn header_name(mut self, name: &'static str) -> Self {
    self.location = ApiKeyLocation::Header(name);
    self
  }

  /// Sets a query parameter name for API key extraction.
  ///
  /// This is a convenience method equivalent to
  /// `.location(ApiKeyLocation::Query(name))`.
  pub fn query_param(mut self, name: &'static str) -> Self {
    self.location = ApiKeyLocation::Query(name);
    self
  }
}

/// Extracts API key from request based on location configuration.
fn extract_api_key(req: &Request, location: &ApiKeyLocation) -> Option<String> {
  match location {
    ApiKeyLocation::Header(name) => req
      .headers()
      .get(*name)
      .and_then(|v| v.to_str().ok())
      .map(|s| s.trim().to_string()),

    ApiKeyLocation::Query(name) => req
      .uri()
      .query()
      .and_then(|q| {
        url::form_urlencoded::parse(q.as_bytes())
          .find(|(k, _)| k == *name)
          .map(|(_, v)| v.to_string())
      }),

    ApiKeyLocation::HeaderOrQuery(header, query) => {
      // Try header first
      if let Some(key) = req
        .headers()
        .get(*header)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
      {
        return Some(key);
      }
      // Fall back to query parameter
      req.uri().query().and_then(|q| {
        url::form_urlencoded::parse(q.as_bytes())
          .find(|(k, _)| k == *query)
          .map(|(_, v)| v.to_string())
      })
    }
  }
}

impl IntoMiddleware for ApiKeyAuth {
  /// Converts the API key authentication configuration into middleware.
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let keys = self.keys.map(Arc::new);
    let verify = self.verify;
    let location = self.location;

    move |req: Request, next: Next| {
      let keys = keys.clone();
      let verify = verify.clone();
      let location = location.clone();

      Box::pin(async move {
        // Extract API key from configured location
        let api_key = match extract_api_key(&req, &location) {
          Some(key) => key,
          None => {
            return http::Response::builder()
              .status(StatusCode::UNAUTHORIZED)
              .header(header::WWW_AUTHENTICATE, "ApiKey")
              .body(TakoBody::new(Full::from(Bytes::from("API key is missing"))))
              .unwrap()
              .into_response();
          }
        };

        // Validate against static keys
        if let Some(set) = &keys {
          if set.contains(&api_key) {
            return next.run(req).await.into_response();
          }
        }

        // Validate using custom verification function
        if let Some(v) = verify.as_ref() {
          if v(&api_key) {
            return next.run(req).await.into_response();
          }
        }

        // Return 401 Unauthorized for invalid keys
        http::Response::builder()
          .status(StatusCode::UNAUTHORIZED)
          .header(header::WWW_AUTHENTICATE, "ApiKey")
          .body(TakoBody::new(Full::from(Bytes::from("Invalid API key"))))
          .unwrap()
          .into_response()
      })
    }
  }
}
