#![cfg_attr(docsrs, doc(cfg(feature = "multipart")))]
//! Multipart form data extraction and file upload handling.
//!
//! This module provides extractors for parsing `multipart/form-data` request bodies,
//! commonly used for file uploads and complex form submissions. It supports both
//! raw multipart access through [`TakoMultipart`](crate::multipart::TakoMultipart) and strongly-typed extraction
//! through [`TakoTypedMultipart`](crate::multipart::TakoTypedMultipart), with built-in support for file uploads to disk
//! or memory.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::multipart::{TakoTypedMultipart, UploadedFile};
//! use serde::Deserialize;
//!
//! #[derive(Deserialize)]
//! struct FileUploadForm {
//!     title: String,
//!     description: String,
//!     file: UploadedFile,
//! }
//!
//! async fn upload_handler(
//!     TakoTypedMultipart { data: form, .. }: TakoTypedMultipart<'_, FileUploadForm, UploadedFile>
//! ) {
//!     println!("Uploaded file: {:?}", form.file.file_name);
//!     println!("File size: {} bytes", form.file.size);
//!     println!("Saved to: {:?}", form.file.path);
//! }
//! ```

mod error;
mod extractor;
mod field;
mod limits;

pub use error::MultipartError;
pub use error::TypedMultipartError;
pub use extractor::TakoMultipart;
pub use extractor::TakoTypedMultipart;
pub use field::BufferedUploadedFile;
pub use field::FromMultipartField;
pub use field::InMemoryFile;
pub use field::TempFileCleanup;
pub use field::UploadedFile;
pub use limits::MultipartConfig;
