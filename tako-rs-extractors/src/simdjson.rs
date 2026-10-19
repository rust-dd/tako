//! JSON bodies parsed by `simd_json` (`SimdJson`, feature `simd-json-impl`) or `sonic_rs`
//! (`SonicJson`, feature `simd-sonic`).
//!
//! # Examples
//!
//! ```
//! use tako::extractors::simdjson::SimdJson;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Deserialize, Serialize)]
//! struct User {
//!     name: String,
//!     email: String,
//!     age: u32,
//! }
//!
//! async fn create_user_handler(SimdJson(user): SimdJson<User>) -> SimdJson<User> {
//!     println!("Creating user: {}", user.name);
//!     // Process user creation...
//!     SimdJson(user)
//! }
//! ```
//!
//! ```
//! use tako::extractors::simdjson::SonicJson;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Deserialize, Serialize)]
//! struct User {
//!     name: String,
//!     email: String,
//!     age: u32,
//! }
//!
//! async fn create_user_handler(SonicJson(user): SonicJson<User>) -> SonicJson<User> {
//!     println!("Creating user: {}", user.name);
//!     // Process user creation...
//!     SonicJson(user)
//! }
//! ```

use http::StatusCode;
use http::header::HeaderValue;
use http::header::{self};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tako_rs_core::body::TakoBody;
use tako_rs_core::extractors::Entries;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::body::collect_body;
use tako_rs_core::extractors::is_json_content_type;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

/// JSON parsed in place by `simd_json`; the route's body limit applies. Also a responder.
///
/// # Examples
///
/// ```
/// use tako::extractors::simdjson::SimdJson;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Deserialize, Serialize)]
/// struct ApiResponse {
///     success: bool,
///     message: String,
/// }
///
/// async fn api_handler(SimdJson(request): SimdJson<ApiResponse>) -> SimdJson<ApiResponse> {
///     // Process the request...
///     SimdJson(ApiResponse {
///         success: true,
///         message: "Request processed successfully".to_string(),
///     })
/// }
/// ```
#[doc(alias = "simdjson")]
#[cfg(feature = "simd-json-impl")]
pub struct SimdJson<T>(pub T);

/// Rejection of the SIMD JSON extractors; a body read error keeps its own status, else 400.
#[derive(Debug)]
pub enum SimdJsonError {
  /// Also returned when the `Content-Type` header is missing.
  InvalidContentType,
  BodyReadError(tako_rs_core::extractors::body::BodyReadError),
  DeserializationError(String),
}

impl Responder for SimdJsonError {
  fn into_response(self) -> Response {
    match self {
      SimdJsonError::InvalidContentType => (
        StatusCode::BAD_REQUEST,
        "Invalid content type; expected JSON",
      )
        .into_response(),
      SimdJsonError::BodyReadError(err) => {
        (err.status(), format!("Failed to read request body: {err}")).into_response()
      }
      SimdJsonError::DeserializationError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Failed to deserialize JSON: {err}"),
      )
        .into_response(),
    }
  }
}

#[cfg(feature = "simd-json-impl")]
impl<'a, T> FromRequest<'a> for SimdJson<T>
where
  T: DeserializeOwned + Send + 'static,
{
  type Error = SimdJsonError;
  const ENTRIES: Entries = Entries::BODY_LIMIT;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      if !is_json_content_type(req.headers()) {
        return Err(SimdJsonError::InvalidContentType);
      }

      let bytes = collect_body(req)
        .await
        .map_err(SimdJsonError::BodyReadError)?;

      // simd_json parses in place; `try_into_mut` skips the copy when the buffer is unshared.
      let mut owned: Vec<u8> = match bytes.try_into_mut() {
        Ok(bm) => bm.into(),
        Err(b) => b.to_vec(),
      };

      let data = simd_json::from_slice::<T>(&mut owned)
        .map_err(|e| SimdJsonError::DeserializationError(e.to_string()))?;

      Ok(SimdJson(data))
    }
  }
}

#[cfg(feature = "simd-json-impl")]
impl<T> Responder for SimdJson<T>
where
  T: Serialize,
{
  /// A serialization error becomes a plain-text 500.
  fn into_response(self) -> Response {
    match simd_json::to_vec(&self.0) {
      Ok(buf) => {
        let mut res = Response::new(TakoBody::from(buf));
        res.headers_mut().insert(
          header::CONTENT_TYPE,
          HeaderValue::from_static(mime::APPLICATION_JSON.as_ref()),
        );
        res
      }
      Err(err) => {
        let mut res = Response::new(TakoBody::from(err.to_string()));
        *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        res.headers_mut().insert(
          header::CONTENT_TYPE,
          HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref()),
        );
        res
      }
    }
  }
}

/// JSON parsed by `sonic_rs`; the route's body limit applies. Also a responder.
///
/// # Examples
///
/// ```
/// use tako::extractors::simdjson::SonicJson;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Deserialize, Serialize)]
/// struct ApiResponse {
///     ok: bool,
/// }
///
/// async fn api_handler(SonicJson(_request): SonicJson<ApiResponse>) -> SonicJson<ApiResponse> {
///     SonicJson(ApiResponse { ok: true })
/// }
/// ```
#[doc(alias = "sonicjson")]
#[cfg(feature = "simd-sonic")]
pub struct SonicJson<T>(pub T);

#[cfg(feature = "simd-sonic")]
impl<'a, T> FromRequest<'a> for SonicJson<T>
where
  T: DeserializeOwned + Send + 'static,
{
  type Error = SimdJsonError;
  const ENTRIES: Entries = Entries::BODY_LIMIT;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      if !is_json_content_type(req.headers()) {
        return Err(SimdJsonError::InvalidContentType);
      }

      let bytes = collect_body(req)
        .await
        .map_err(SimdJsonError::BodyReadError)?;

      let data = sonic_rs::from_slice::<T>(&bytes)
        .map_err(|e| SimdJsonError::DeserializationError(e.to_string()))?;

      Ok(SonicJson(data))
    }
  }
}

#[cfg(feature = "simd-sonic")]
impl<T> Responder for SonicJson<T>
where
  T: Serialize,
{
  /// A serialization error becomes a plain-text 500.
  fn into_response(self) -> Response {
    match sonic_rs::to_vec(&self.0) {
      Ok(buf) => {
        let mut res = Response::new(TakoBody::from(buf));
        res.headers_mut().insert(
          header::CONTENT_TYPE,
          HeaderValue::from_static(mime::APPLICATION_JSON.as_ref()),
        );
        res
      }
      Err(err) => {
        let mut res = Response::new(TakoBody::from(err.to_string()));
        *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        res.headers_mut().insert(
          header::CONTENT_TYPE,
          HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref()),
        );
        res
      }
    }
  }
}
