//! HTTP Range header extraction for partial content requests.
//!
//! This module provides extractors for parsing HTTP Range headers into strongly-typed Rust
//! structures. Range requests are commonly used for resumable downloads, streaming media,
//! and serving large files in chunks. The extractor validates Range header format, parses
//! byte ranges, and provides detailed error information for malformed range specifications.
//! It supports the standard `bytes=start-end` format used by browsers and download managers.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::range::Range;
//! use tako::extractors::FromRequest;
//! use tako::types::Request;
//!
//! async fn serve_partial_file(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
//!     let range: Option<Range> = Option::<Range>::from_request(&mut req).await?;
//!
//!     match range {
//!         Some(range) => {
//!             println!("Serving bytes {}-{}", range.start, range.end);
//!             // Serve the specified byte range of the file
//!             Ok(format!("Partial content: bytes {}-{}", range.start, range.end))
//!         }
//!         None => {
//!             println!("Serving full file");
//!             // Serve the complete file
//!             Ok("Full file content".to_string())
//!         }
//!     }
//! }
//!
//! // Example with explicit range handling
//! async fn download_handler(mut req: Request) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
//!     let range: Option<Range> = Option::<Range>::from_request(&mut req).await?;
//!     let file_data = load_file_data(); // Your file loading logic
//!
//!     match range {
//!         Some(Range { start, end }) => {
//!             let file_size = file_data.len() as u64;
//!             let actual_end = if end == 0 { file_size - 1 } else { end.min(file_size - 1) };
//!
//!             if start <= actual_end && start < file_size {
//!                 Ok(file_data[start as usize..=actual_end as usize].to_vec())
//!             } else {
//!                 Err("Invalid range".into())
//!             }
//!         }
//!         None => Ok(file_data),
//!     }
//! }
//!
//! fn load_file_data() -> Vec<u8> {
//!     // Mock file data
//!     vec![0; 1024]
//! }
//! ```

use http::HeaderMap;
use http::StatusCode;
use http::request::Parts;

use crate::extractors::FromRequest;
use crate::extractors::FromRequestParts;
use crate::responder::Responder;
use crate::types::Request;

/// Extracted byte range for HTTP partial content requests.
#[derive(Debug, Clone, Copy)]
#[doc(alias = "range")]
pub struct Range {
  /// The starting byte position (0-based, inclusive).
  pub start: u64,
  /// The ending byte position (0-based, inclusive).
  /// If 0, it typically means "to the end of the file".
  pub end: u64,
}

/// Error type for Range header extraction and parsing.
#[derive(Debug)]
pub enum RangeError {
  /// Range header is not present in the request.
  Missing,
  /// Range header format is invalid (doesn't start with "bytes=" or malformed range).
  InvalidFormat,
  /// Numeric values in the range could not be parsed (invalid numbers).
  ParseError,
}

impl Responder for RangeError {
  /// Converts Range extraction errors into HTTP 416 Range Not Satisfiable responses.
  fn into_response(self) -> crate::types::Response {
    match self {
      RangeError::Missing => {
        (StatusCode::RANGE_NOT_SATISFIABLE, "Missing Range header").into_response()
      }
      RangeError::InvalidFormat => (
        StatusCode::RANGE_NOT_SATISFIABLE,
        "Invalid Range format. Expected: bytes=start-end",
      )
        .into_response(),
      RangeError::ParseError => (
        StatusCode::RANGE_NOT_SATISFIABLE,
        "Failed to parse numeric values from Range",
      )
        .into_response(),
    }
  }
}

impl Range {
  /// Parses the Range header value in `bytes=start-end` format.
  pub fn from_headers(headers: &HeaderMap) -> Result<Option<Self>, RangeError> {
    let value = match headers.get("range") {
      Some(v) => v.to_str().map_err(|_| RangeError::InvalidFormat)?,
      None => return Ok(None),
    };

    if !value.starts_with("bytes=") {
      return Err(RangeError::InvalidFormat);
    }

    let range = &value["bytes=".len()..];
    let mut parts = range.splitn(2, '-');

    let start_str = parts.next().ok_or(RangeError::InvalidFormat)?;
    let end_str = parts.next().ok_or(RangeError::InvalidFormat)?;

    let start = start_str
      .parse::<u64>()
      .map_err(|_| RangeError::ParseError)?;
    let end = if end_str.is_empty() {
      0
    } else {
      end_str.parse::<u64>().map_err(|_| RangeError::ParseError)?
    };

    Ok(Some(Self { start, end }))
  }
}

impl<'a> FromRequest<'a> for Option<Range> {
  type Error = RangeError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Range::from_headers(req.headers()))
  }
}

impl<'a> FromRequestParts<'a> for Option<Range> {
  type Error = RangeError;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Range::from_headers(&parts.headers))
  }
}
