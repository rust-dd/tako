//! Validation wrappers — opt-in via `validator` / `garde` cargo features.
//!
//! These wrappers compose with any other extractor: extract `T`, then run the
//! validator before handing it to the handler. Validation failures produce
//! 422 with an `application/problem+json` body that lists the violated rules.
//!
//! Two wrappers are exposed so callers can pick a validator deterministically
//! when both crates are linked:
//!
//! - [`Validated<T>`](crate::validate::Validated) — uses [`validator::Validate`] when the `validator`
//!   feature is on, falls back to [`garde::Validate`] when only `garde` is on.
//! - [`Garded<T>`](crate::validate::Garded) — always uses [`garde::Validate`]. Available whenever the
//!   `garde` feature is on, regardless of whether `validator` is also active.
//!
//! Previously enabling both features silently dropped the `garde` blanket
//! impl: callers thought their `#[derive(garde::Validate)]` types were being
//! checked when in fact only `validator`-annotated rules ran. Use
//! [`Garded<T>`](crate::validate::Garded) to make the choice explicit.
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
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;

/// Wraps an inner extractor and runs `Validate::validate` on the produced value.
pub struct Validated<T>(pub T);

/// Trait abstracting `validator::Validate` and `garde::Validate` so the same
/// wrapper supports both crates. Implemented automatically when either feature
/// is enabled.
pub trait Validate {
  /// Validation error message — already human-readable, content-type `application/problem+json`-friendly.
  type Error: std::fmt::Display;

  /// Run the validation rules on `&self`.
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

/// Rejection variants for [`Validated`].
#[derive(Debug)]
pub enum ValidatedError<E> {
  /// Inner extractor failed.
  Inner(E),
  /// The deserialized value violated one or more validation rules.
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
        let mut res = http::Response::builder()
          .status(StatusCode::UNPROCESSABLE_ENTITY)
          .header(CONTENT_TYPE, "application/problem+json")
          .body(tako_rs_core::body::TakoBody::from(body.to_string()))
          .expect("valid problem+json response");
        // No-op: Responder for tuple already builds Response, but we want explicit
        // problem+json so we built it manually above.
        let _ = &mut res;
        res
      }
    }
  }
}

impl<'a, T> FromRequest<'a> for Validated<T>
where
  T: FromRequest<'a> + Validate + Send + 'a,
{
  type Error = ValidatedError<<T as FromRequest<'a>>::Error>;

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

// -- garde-specific wrapper -------------------------------------------------

/// Wraps an inner extractor and runs [`garde::Validate`] on the produced
/// value. Use when both `validator` and `garde` features are enabled and you
/// want the garde rules to run; otherwise prefer the generic [`Validated<T>`].
#[cfg(feature = "garde")]
#[cfg_attr(docsrs, doc(cfg(feature = "garde")))]
pub struct Garded<T>(pub T);

#[cfg(feature = "garde")]
impl<'a, T> FromRequest<'a> for Garded<T>
where
  T: FromRequest<'a> + garde::Validate<Context = ()> + Send + 'a,
{
  type Error = ValidatedError<<T as FromRequest<'a>>::Error>;

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
