//! `Validated<T>` checks an extracted `T` with `validator`, or with `garde` when only that is
//! enabled; `Garded<T>` always uses `garde`. A violation responds 422 `application/problem+json`.
//!
//! # Examples
//!
//! ```rust,ignore
//! use serde::Deserialize;
//! use validator::Validate;
//! use tako::extractors::validate::Validated;
//! use tako::extractors::json::Json;
//!
//! #[derive(Deserialize, Validate)]
//! struct CreateUser {
//!     #[validate(email)]
//!     email: String,
//!     #[validate(length(min = 8))]
//!     password: String,
//! }
//!
//! async fn handler(Validated(Json(payload)): Validated<Json<CreateUser>>) {
//!     println!("valid {}", payload.email);
//! }
//! ```

use http::StatusCode;
use http::header::CONTENT_TYPE;
use http::request::Parts;
use tako_rs_core::extractors::Entries;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;

pub struct Validated<T>(pub T);

/// Implemented for every `validator::Validate` type, or every `garde::Validate` type when only
/// `garde` is enabled.
pub trait Validate {
  /// Its `Display` text becomes the problem `detail`.
  type Error: std::fmt::Display;

  fn validate(&self) -> Result<(), Self::Error>;
}

#[cfg(feature = "validator")]
#[cfg_attr(docsrs, doc(cfg(feature = "validator")))]
impl<T> Validate for T
where
  T: validator::Validate,
{
  type Error = validator::ValidationErrors;

  fn validate(&self) -> Result<(), Self::Error> {
    validator::Validate::validate(self)
  }
}

#[cfg(all(feature = "garde", not(feature = "validator")))]
#[cfg_attr(docsrs, doc(cfg(feature = "garde")))]
impl<T> Validate for T
where
  T: garde::Validate<Context = ()>,
{
  type Error = garde::Report;

  fn validate(&self) -> Result<(), Self::Error> {
    garde::Validate::validate(self)
  }
}

/// Rejection of [`Validated`] and `Garded`.
#[derive(Debug)]
pub enum ValidatedError<E> {
  Inner(E),
  /// Responds 422 with this text as the problem `detail`.
  Failed(String),
}

impl<E: Responder> Responder for ValidatedError<E> {
  fn into_response(self) -> tako_rs_core::types::Response {
    match self {
      Self::Inner(e) => e.into_response(),
      Self::Failed(detail) => {
        let body = serde_json::json!({
          "type": "about:blank",
          "title": "Unprocessable Entity",
          "status": 422_u16,
          "detail": detail,
        });
        http::Response::builder()
          .status(StatusCode::UNPROCESSABLE_ENTITY)
          .header(CONTENT_TYPE, "application/problem+json")
          .body(tako_rs_core::body::TakoBody::from(body.to_string()))
          .expect("valid problem+json response")
      }
    }
  }
}

impl<'a, T> FromRequest<'a> for Validated<T>
where
  T: FromRequest<'a> + Validate + Send + 'a,
{
  type Error = ValidatedError<<T as FromRequest<'a>>::Error>;
  const ENTRIES: Entries = <T as FromRequest<'a>>::ENTRIES;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      let inner = T::from_request(req).await.map_err(ValidatedError::Inner)?;
      Validate::validate(&inner)
        .map_err(|e| ValidatedError::Failed(e.to_string()))
        .map(|()| Validated(inner))
    }
  }
}

impl<'a, T> FromRequestParts<'a> for Validated<T>
where
  T: FromRequestParts<'a> + Validate + Send + 'a,
{
  type Error = ValidatedError<<T as FromRequestParts<'a>>::Error>;
  const ENTRIES: Entries = <T as FromRequestParts<'a>>::ENTRIES;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      let inner = T::from_request_parts(parts)
        .await
        .map_err(ValidatedError::Inner)?;
      Validate::validate(&inner)
        .map_err(|e| ValidatedError::Failed(e.to_string()))
        .map(|()| Validated(inner))
    }
  }
}

/// Checks with `garde` even when `validator` is also enabled.
#[cfg(feature = "garde")]
#[cfg_attr(docsrs, doc(cfg(feature = "garde")))]
pub struct Garded<T>(pub T);

#[cfg(feature = "garde")]
impl<'a, T> FromRequest<'a> for Garded<T>
where
  T: FromRequest<'a> + garde::Validate<Context = ()> + Send + 'a,
{
  type Error = ValidatedError<<T as FromRequest<'a>>::Error>;
  const ENTRIES: Entries = <T as FromRequest<'a>>::ENTRIES;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      let inner = T::from_request(req).await.map_err(ValidatedError::Inner)?;
      garde::Validate::validate(&inner)
        .map_err(|e| ValidatedError::Failed(e.to_string()))
        .map(|()| Garded(inner))
    }
  }
}

#[cfg(feature = "garde")]
impl<'a, T> FromRequestParts<'a> for Garded<T>
where
  T: FromRequestParts<'a> + garde::Validate<Context = ()> + Send + 'a,
{
  type Error = ValidatedError<<T as FromRequestParts<'a>>::Error>;
  const ENTRIES: Entries = <T as FromRequestParts<'a>>::ENTRIES;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      let inner = T::from_request_parts(parts)
        .await
        .map_err(ValidatedError::Inner)?;
      garde::Validate::validate(&inner)
        .map_err(|e| ValidatedError::Failed(e.to_string()))
        .map(|()| Garded(inner))
    }
  }
}
