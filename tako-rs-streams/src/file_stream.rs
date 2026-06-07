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

mod conditional;
mod date;
mod etag;
mod stream;

pub use conditional::evaluate_conditional;
pub use etag::weak_etag_from_metadata;
pub use stream::FileStream;
