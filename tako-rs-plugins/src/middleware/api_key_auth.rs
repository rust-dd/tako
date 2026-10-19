//! API key authentication: a key passes when it matches a static key (compared in constant
//! time) or the verify callback accepts it.
//!
//! # Examples
//!
//! ```rust,ignore
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

use std::borrow::Cow;
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

/// Scans every candidate; timing can reveal a key's length but not its contents.
pub(super) fn constant_time_contains(input: &[u8], candidates: &[Vec<u8>]) -> bool {
  let mut found = Choice::from(0u8);
  for candidate in candidates {
    found |= input.ct_eq(candidate.as_slice());
  }
  bool::from(found)
}

/// Defaults to the `X-API-Key` header.
#[derive(Clone)]
pub enum ApiKeyLocation {
  Header(&'static str),
  Query(&'static str),
  /// Tries the header first, then the query parameter.
  HeaderOrQuery(&'static str, &'static str),
}

impl Default for ApiKeyLocation {
  fn default() -> Self {
    Self::Header("X-API-Key")
  }
}

/// Custom verification closure for [`ApiKeyAuth`].
pub type ApiKeyVerifyFn = Arc<dyn Fn(&str) -> bool + Send + Sync + 'static>;

/// Rejects a missing or unknown key with 401 and `WWW-Authenticate: ApiKey`.
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
  keys: Option<Vec<Vec<u8>>>,
  verify: Option<ApiKeyVerifyFn>,
  location: ApiKeyLocation,
}

impl ApiKeyAuth {
  pub fn new(key: impl Into<String>) -> Self {
    let key: String = key.into();
    Self {
      keys: Some(vec![key.into_bytes()]),
      verify: None,
      location: ApiKeyLocation::default(),
    }
  }

  pub fn from_keys<I>(keys: I) -> Self
  where
    I: IntoIterator,
    I::Item: Into<String>,
  {
    Self {
      keys: Some(
        keys
          .into_iter()
          .map(|k| Into::<String>::into(k).into_bytes())
          .collect(),
      ),
      verify: None,
      location: ApiKeyLocation::default(),
    }
  }

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

  pub fn from_keys_with_verify<I, F>(keys: I, f: F) -> Self
  where
    I: IntoIterator,
    I::Item: Into<String>,
    F: Fn(&str) -> bool + Send + Sync + 'static,
  {
    Self {
      keys: Some(
        keys
          .into_iter()
          .map(|k| Into::<String>::into(k).into_bytes())
          .collect(),
      ),
      verify: Some(Arc::new(f)),
      location: ApiKeyLocation::default(),
    }
  }

  pub fn location(mut self, location: ApiKeyLocation) -> Self {
    self.location = location;
    self
  }

  pub fn header_name(mut self, name: &'static str) -> Self {
    self.location = ApiKeyLocation::Header(name);
    self
  }

  pub fn query_param(mut self, name: &'static str) -> Self {
    self.location = ApiKeyLocation::Query(name);
    self
  }
}

fn header_key<'a>(req: &'a Request, name: &str) -> Option<Cow<'a, str>> {
  req
    .headers()
    .get(name)
    .and_then(|v| v.to_str().ok())
    .map(|s| Cow::Borrowed(s.trim()))
}

fn query_key<'a>(req: &'a Request, name: &str) -> Option<Cow<'a, str>> {
  req.uri().query().and_then(|q| {
    url::form_urlencoded::parse(q.as_bytes())
      .find(|(k, _)| k == name)
      .map(|(_, v)| v)
  })
}

fn extract_api_key<'a>(req: &'a Request, location: &ApiKeyLocation) -> Option<Cow<'a, str>> {
  match location {
    ApiKeyLocation::Header(name) => header_key(req, name),
    ApiKeyLocation::Query(name) => query_key(req, name),
    ApiKeyLocation::HeaderOrQuery(header, query) => {
      header_key(req, header).or_else(|| query_key(req, query))
    }
  }
}

impl IntoMiddleware for ApiKeyAuth {
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
    let api_key_authenticate = HeaderValue::from_static("ApiKey");

    move |req: Request, next: Next| {
      let keys = keys.clone();
      let verify = verify.clone();
      let location = location.clone();
      let api_key_authenticate = api_key_authenticate.clone();

      Box::pin(async move {
        let Some(api_key) = extract_api_key(&req, &location) else {
          return http::Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header(header::WWW_AUTHENTICATE, api_key_authenticate.clone())
            .body(TakoBody::from("API key is missing"))
            .unwrap()
            .into_response();
        };

        if let Some(set) = &keys
          && constant_time_contains(api_key.as_bytes(), set)
        {
          return next.run(req).await.into_response();
        }

        if let Some(v) = verify.as_ref()
          && v(api_key.as_ref())
        {
          return next.run(req).await.into_response();
        }

        http::Response::builder()
          .status(StatusCode::UNAUTHORIZED)
          .header(header::WWW_AUTHENTICATE, api_key_authenticate)
          .body(TakoBody::from("Invalid API key"))
          .unwrap()
          .into_response()
      })
    }
  }
}
