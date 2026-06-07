//! Static file serving utilities for web applications.
//!
//! `ServeDir` serves files from a directory tree with index resolution,
//! precompressed-asset preference (`*.br` / `*.gz`), an SPA fallback rewrite,
//! and a canonicalize + prefix-check guard against path traversal.
//!
//! `ServeFile` serves a single file.

mod dir;
mod file;
mod serve;

pub use dir::PrecompressedPolicy;
pub use dir::ServeDir;
pub use dir::ServeDirBuilder;
pub use file::ServeFile;
pub use file::ServeFileBuilder;
