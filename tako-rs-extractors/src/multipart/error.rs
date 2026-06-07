use http::StatusCode;
use tako_rs_core::responder::Responder;

/// Error type for multipart extraction.
#[derive(Debug)]
pub enum MultipartError {
  /// Content-Type header is missing from the request.
  MissingContentType,
  /// Content-Type header is not multipart/form-data.
  InvalidContentType,
  /// Content-Type header contains invalid UTF-8 sequences.
  InvalidUtf8,
  /// Failed to parse boundary from Content-Type header.
  BoundaryParseError(String),
  /// A part's content-type is not in the configured allow-list.
  DisallowedContentType(String),
  /// The configured `max_parts` count was exceeded.
  TooManyParts,
}

impl Responder for MultipartError {
  /// Converts the error into an HTTP response.
  fn into_response(self) -> tako_rs_core::types::Response {
    match self {
      MultipartError::MissingContentType => {
        (StatusCode::BAD_REQUEST, "Missing Content-Type header").into_response()
      }
      MultipartError::InvalidContentType => {
        (StatusCode::BAD_REQUEST, "Invalid Content-Type header").into_response()
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
      MultipartError::DisallowedContentType(ct) => (
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        format!("part content-type not allowed: {ct}"),
      )
        .into_response(),
      MultipartError::TooManyParts => (
        StatusCode::PAYLOAD_TOO_LARGE,
        "too many multipart parts in request",
      )
        .into_response(),
    }
  }
}

/// Error type for typed multipart extraction.
#[derive(Debug)]
pub enum TypedMultipartError {
  /// Content-Type header is missing from the request.
  MissingContentType,
  /// Content-Type header is not multipart/form-data.
  InvalidContentType,
  /// Content-Type header contains invalid UTF-8 sequences.
  InvalidUtf8,
  /// Failed to parse boundary from Content-Type header.
  BoundaryParseError(String),
  /// Error processing a multipart field.
  FieldError(String),
  /// Failed to deserialize form data into the target type.
  DeserializationError(String),
  /// I/O error occurred during processing.
  IoError(String),
  /// A part's content-type is not in the configured allow-list.
  DisallowedContentType(String),
  /// The configured `max_parts` count was exceeded.
  TooManyParts,
}

impl Responder for TypedMultipartError {
  /// Converts the error into an HTTP response.
  fn into_response(self) -> tako_rs_core::types::Response {
    match self {
      TypedMultipartError::MissingContentType => {
        (StatusCode::BAD_REQUEST, "Missing Content-Type header").into_response()
      }
      TypedMultipartError::InvalidContentType => {
        (StatusCode::BAD_REQUEST, "Invalid Content-Type header").into_response()
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
      TypedMultipartError::DeserializationError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Deserialization error: {err}"),
      )
        .into_response(),
      TypedMultipartError::IoError(err) => (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("IO error: {err}"),
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
