//! Path extraction from HTTP requests.
//!
//! This module provides the [`Path`](crate::path::Path) extractor for accessing the URI path from
//! incoming HTTP requests. It wraps a reference to the path string, allowing
//! efficient access to the request path without copying the underlying data.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::path::Path;
//! use tako::types::Request;
//!
//! async fn handle_path(Path(path): Path<'_>) {
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
use tako_core::extractors::FromRequest;
use tako_core::extractors::FromRequestParts;
use tako_core::extractors::params::Params;
use tako_core::extractors::params::ParamsError;
use tako_core::types::Request;

/// Owned URI-path extractor.
///
/// Returns the request path verbatim — no captures, no decoding. For typed
/// path parameters use [`Path<T>`] (axum parity, generic over `T`).
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

/// Typed path-parameter extractor (axum parity).
///
/// `T` may be a single primitive (`Path<u64>`), a tuple (`Path<(u64, String)>`),
/// a `Vec<T>` for repeated captures, an `Option<T>` (`None` when no captures
/// matched), or a struct deriving `serde::Deserialize`.
///
/// Internally delegates to the path-params deserializer in `tako-core`, which
/// has been extended in v2 to support tuples, sequences, and primitive
/// destructuring on top of the original struct/map mode.
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
