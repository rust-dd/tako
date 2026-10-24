use http::StatusCode;
use tako_rs_core::responder::Responder;

/// Rejection of [`TakoMultipart`](crate::multipart::TakoMultipart).
#[derive(Debug)]
pub enum MultipartError {
  MissingContentType,
  /// The `Content-Type` header is not visible ASCII.
  InvalidUtf8,
  /// The `Content-Type` is not `multipart/form-data` or lacks a boundary.
  BoundaryParseError(String),
  /// A field failed to parse or exceeded a size limit.
  FieldError(multer::Error),
  /// The configured `max_parts` count was exceeded.
  TooManyParts,
  /// A part's content type is not in the configured allow-list.
  DisallowedContentType(String),
}

impl Responder for MultipartError {
  fn into_response(self) -> tako_rs_core::types::Response {
    match self {
      MultipartError::MissingContentType => {
        (StatusCode::BAD_REQUEST, "Missing Content-Type header").into_response()
      }
      MultipartError::InvalidUtf8 => (
        StatusCode::BAD_REQUEST,
        "Content-Type header contains invalid UTF-8",
      )
        .into_response(),
      MultipartError::BoundaryParseError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Not multipart/form-data or boundary missing: {err}"),
      )
        .into_response(),
      MultipartError::FieldError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Field processing error: {err}"),
      )
        .into_response(),
      MultipartError::TooManyParts => (
        StatusCode::PAYLOAD_TOO_LARGE,
        "too many multipart parts in request",
      )
        .into_response(),
      MultipartError::DisallowedContentType(ct) => (
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        format!("part content-type not allowed: {ct}"),
      )
        .into_response(),
    }
  }
}

impl std::fmt::Display for MultipartError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::MissingContentType => f.write_str("missing Content-Type header"),
      Self::InvalidUtf8 => f.write_str("invalid Content-Type header encoding"),
      Self::BoundaryParseError(error) => write!(f, "invalid multipart boundary: {error}"),
      Self::FieldError(error) => write!(f, "multipart field error: {error}"),
      Self::TooManyParts => f.write_str("too many multipart parts in request"),
      Self::DisallowedContentType(content_type) => {
        write!(f, "part content-type not allowed: {content_type}")
      }
    }
  }
}

impl std::error::Error for MultipartError {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    match self {
      Self::FieldError(error) => Some(error),
      _ => None,
    }
  }
}

/// Rejection of [`TakoTypedMultipart`](crate::multipart::TakoTypedMultipart).
#[derive(Debug)]
pub enum TypedMultipartError {
  MissingContentType,
  /// The `Content-Type` header is not visible ASCII, or a text field is not UTF-8.
  InvalidUtf8,
  /// The `Content-Type` is not `multipart/form-data` or lacks a boundary.
  BoundaryParseError(String),
  /// A field failed to parse, exceeded a size limit, or timed out.
  FieldError(String),
  /// The body exceeds the shared buffering cap or the multipart whole-request cap.
  BodyTooLarge,
  /// The collected fields do not deserialize into the target type.
  DeserializationError(String),
  /// A part's content type is not in the configured allow-list.
  DisallowedContentType(String),
  /// The configured `max_parts` count was exceeded.
  TooManyParts,
}

impl TypedMultipartError {
  pub(super) fn from_field(error: impl Into<anyhow::Error>) -> Self {
    let error = error.into();
    if error.chain().any(|source| {
      matches!(
        source.downcast_ref::<multer::Error>(),
        Some(multer::Error::StreamSizeExceeded { .. })
      )
    }) {
      Self::BodyTooLarge
    } else {
      Self::FieldError(error.to_string())
    }
  }
}

impl Responder for TypedMultipartError {
  fn into_response(self) -> tako_rs_core::types::Response {
    match self {
      TypedMultipartError::MissingContentType => {
        (StatusCode::BAD_REQUEST, "Missing Content-Type header").into_response()
      }
      TypedMultipartError::InvalidUtf8 => (
        StatusCode::BAD_REQUEST,
        "Content-Type header contains invalid UTF-8",
      )
        .into_response(),
      TypedMultipartError::BoundaryParseError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Not multipart/form-data or boundary missing: {err}"),
      )
        .into_response(),
      TypedMultipartError::FieldError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Field processing error: {err}"),
      )
        .into_response(),
      TypedMultipartError::BodyTooLarge => (
        StatusCode::PAYLOAD_TOO_LARGE,
        "multipart body exceeds the configured limit",
      )
        .into_response(),
      TypedMultipartError::DeserializationError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Deserialization error: {err}"),
      )
        .into_response(),
      TypedMultipartError::DisallowedContentType(ct) => (
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        format!("part content-type not allowed: {ct}"),
      )
        .into_response(),
      TypedMultipartError::TooManyParts => (
        StatusCode::PAYLOAD_TOO_LARGE,
        "too many multipart parts in request",
      )
        .into_response(),
    }
  }
}
