//! `application/x-www-form-urlencoded` bodies deserialized with `serde_urlencoded`.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::form::Form;
//! use serde::Deserialize;
//!
//! #[derive(Deserialize)]
//! struct LoginForm {
//!     username: String,
//!     password: String,
//! }
//!
//! async fn login_handler(Form(form): Form<LoginForm>) {
//!     println!("Username: {}", form.username);
//!     // Handle login logic...
//! }
//! ```

use http::StatusCode;
use serde::de::DeserializeOwned;
use tako_rs_core::extractors::Entries;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::body::collect_body;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;

/// A form body as `T`; the route's body limit applies.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::form::Form;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct ContactForm {
///     name: String,
///     email: String,
///     message: String,
/// }
///
/// async fn contact_handler(Form(contact): Form<ContactForm>) {
///     println!("Received message from {} ({}): {}",
///              contact.name, contact.email, contact.message);
/// }
/// ```
#[doc(alias = "form")]
pub struct Form<T>(pub T);

/// Rejection of [`Form`]; a body read error keeps its own status, everything else is 400.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormError {
  /// The `Content-Type` does not start with `application/x-www-form-urlencoded`.
  InvalidContentType,
  BodyReadError(tako_rs_core::extractors::body::BodyReadError),
  InvalidUtf8,
  DeserializationError(String),
}

impl std::fmt::Display for FormError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::InvalidContentType => {
        write!(
          f,
          "invalid content type; expected application/x-www-form-urlencoded"
        )
      }
      Self::BodyReadError(err) => write!(f, "failed to read request body: {err}"),
      Self::InvalidUtf8 => write!(f, "request body contains invalid UTF-8"),
      Self::DeserializationError(err) => write!(f, "failed to deserialize form data: {err}"),
    }
  }
}

impl std::error::Error for FormError {}

impl Responder for FormError {
  /// # Examples
  ///
  /// ```rust
  /// use tako::extractors::form::FormError;
  /// use tako::responder::Responder;
  /// use http::StatusCode;
  ///
  /// let error = FormError::InvalidContentType;
  /// let response = error.into_response();
  /// assert_eq!(response.status(), StatusCode::BAD_REQUEST);
  ///
  /// let error = FormError::InvalidUtf8;
  /// let response = error.into_response();
  /// assert_eq!(response.status(), StatusCode::BAD_REQUEST);
  /// ```
  fn into_response(self) -> tako_rs_core::types::Response {
    match self {
      FormError::InvalidContentType => (
        StatusCode::BAD_REQUEST,
        "Invalid content type; expected application/x-www-form-urlencoded",
      )
        .into_response(),
      FormError::BodyReadError(err) => {
        (err.status(), format!("Failed to read request body: {err}")).into_response()
      }
      FormError::InvalidUtf8 => (
        StatusCode::BAD_REQUEST,
        "Request body contains invalid UTF-8",
      )
        .into_response(),
      FormError::DeserializationError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Failed to deserialize form data: {err}"),
      )
        .into_response(),
    }
  }
}

impl<'a, T> FromRequest<'a> for Form<T>
where
  T: DeserializeOwned + Send + 'static,
{
  type Error = FormError;
  const ENTRIES: Entries = Entries::BODY_LIMIT;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      // A prefix match admits `; charset=utf-8`, the same check `FormBorrowed` uses.
      let content_type = req
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

      if !content_type.starts_with("application/x-www-form-urlencoded") {
        return Err(FormError::InvalidContentType);
      }

      let body_bytes = collect_body(req).await.map_err(FormError::BodyReadError)?;
      let body_str = std::str::from_utf8(&body_bytes).map_err(|_| FormError::InvalidUtf8)?;
      let form_data = serde_urlencoded::from_str::<T>(body_str)
        .map_err(|e| FormError::DeserializationError(e.to_string()))?;

      Ok(Form(form_data))
    }
  }
}
