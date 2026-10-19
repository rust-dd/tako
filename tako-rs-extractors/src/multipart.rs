//! `multipart/form-data` bodies, as raw fields through [`TakoMultipart`] or as a typed struct
//! through [`TakoTypedMultipart`], with file fields kept in memory or saved to disk.
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
mod stream;

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
pub use stream::MultipartFields;
