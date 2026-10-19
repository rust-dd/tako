//! Streamed file responses, plus the precondition and `ETag` helpers behind them.
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

mod stream;

pub use stream::FileStream;

pub use crate::file_io::FileByteStream;
pub use crate::file_io::conditional::evaluate_conditional;
pub use crate::file_io::conditional::evaluate_conditional_for_method;
pub use crate::file_io::etag::weak_etag_from_metadata;
