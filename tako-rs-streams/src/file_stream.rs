//! File streaming utilities for efficient HTTP file delivery.
//!
//! This module provides `FileStream` for streaming files over HTTP with support for
//! range requests, content-length headers, and proper MIME type detection. It enables
//! efficient delivery of large files without loading them entirely into memory, making
//! it suitable for serving media files, downloads, and other binary content.
//!
//! # Examples
//!
//! ```rust,ignore
//! use tako::file_stream::FileStream;
//! use tako::responder::Responder;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // `Stream` a file from disk
//! let file_stream = FileStream::from_path("./assets/video.mp4").await?;
//! let response = file_stream.into_response();
//! # Ok(())
//! # }
//! ```

#![cfg_attr(docsrs, doc(cfg(feature = "file-stream")))]

mod stream;

pub use stream::FileStream;

pub use crate::file_io::FileByteStream;
pub use crate::file_io::conditional::evaluate_conditional;
pub use crate::file_io::conditional::evaluate_conditional_for_method;
pub use crate::file_io::etag::weak_etag_from_metadata;
