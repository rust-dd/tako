//! Typed path parameters ([`Path`]) and the raw URI path ([`RawPath`]).
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::path::RawPath;
//! use tako::types::Request;
//!
//! async fn handle_path(RawPath(path): RawPath) {
//!     println!("Request path: {}", path);
//!
//!     // Check specific path patterns
//!     if path.starts_with("/api/") {
//!         println!("API endpoint");
//!     }
//!
//!     // Extract path segments
//!     let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
//!     println!("Path segments: {:?}", segments);
//! }
//! ```

use std::convert::Infallible;

use http::request::Parts;
use serde::de::DeserializeOwned;
use tako_rs_core::extractors::Entries;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::extractors::params::Params;
use tako_rs_core::extractors::params::ParamsError;
use tako_rs_core::types::Request;

/// The request path verbatim, without percent-decoding.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::path::RawPath;
/// use tako::types::Request;
///
/// async fn handler(RawPath(path): RawPath) {
///     match path.as_str() {
///         "/health" => println!("Health check endpoint"),
///         "/api/users" => println!("Users API endpoint"),
///         _ if path.starts_with("/static/") => println!("Static file request"),
///         _ => println!("Other path: {}", path),
///     }
/// }
/// ```
#[doc(alias = "raw-path")]
pub struct RawPath(pub String);

impl<'a> FromRequest<'a> for RawPath {
  type Error = Infallible;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Ok(RawPath(req.uri().path().to_string())))
  }
}

impl<'a> FromRequestParts<'a> for RawPath {
  type Error = Infallible;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Ok(RawPath(parts.uri.path().to_string())))
  }
}

/// Path parameters as `T`: a primitive, a tuple, a `Vec`, an `Option` (`None` without
/// captures), or a `Deserialize` struct.
///
/// # Examples
///
/// ```rust,ignore
/// use tako::extractors::path::Path;
///
/// // Single primitive
/// async fn by_id(Path(id): Path<u64>) -> String { format!("id={id}") }
///
/// // Tuple
/// async fn pair(Path((a, b)): Path<(String, u32)>) -> String { format!("{a}/{b}") }
///
/// // Struct
/// #[derive(serde::Deserialize)]
/// struct UserKey { tenant: String, user_id: u64 }
/// async fn user(Path(key): Path<UserKey>) -> String {
///   format!("{}:{}", key.tenant, key.user_id)
/// }
/// ```
#[doc(alias = "path")]
pub struct Path<T>(pub T);

impl<'a, T> FromRequest<'a> for Path<T>
where
  T: DeserializeOwned + Send + 'a,
{
  type Error = ParamsError;
  const ENTRIES: Entries = Entries::PARAMS;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move { Params::<T>::from_request(req).await.map(|p| Path(p.0)) }
  }
}

impl<'a, T> FromRequestParts<'a> for Path<T>
where
  T: DeserializeOwned + Send + 'a,
{
  type Error = ParamsError;
  const ENTRIES: Entries = Entries::PARAMS;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      Params::<T>::from_request_parts(parts)
        .await
        .map(|p| Path(p.0))
    }
  }
}
