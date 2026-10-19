//! `ServeDir` for a directory tree, with index files, `.br`/`.gz` sidecars, an SPA fallback and
//! traversal checks, and `ServeFile` for one file.

mod dir;
mod file;
mod serve;

pub use dir::PrecompressedPolicy;
pub use dir::ServeDir;
pub use dir::ServeDirBuilder;
pub use file::ServeFile;
pub use file::ServeFileBuilder;
