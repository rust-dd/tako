use std::path::PathBuf;

use http::HeaderValue;
use http::StatusCode;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

use crate::file_io::OpenFile;
use crate::file_io::response;

/// A single streaming file responder with cache and byte-range support.
#[doc(alias = "serve_file")]
pub struct ServeFile {
  path: PathBuf,
  cache_control: Option<HeaderValue>,
}

/// Configure a single file responder.
#[must_use]
pub struct ServeFileBuilder {
  file: ServeFile,
}

impl ServeFileBuilder {
  /// Set the trusted file path.
  pub fn new(path: impl Into<PathBuf>) -> Self {
    Self {
      file: ServeFile {
        path: path.into(),
        cache_control: None,
      },
    }
  }

  /// Set Cache-Control on successful and not-modified responses.
  pub fn cache_control(mut self, value: HeaderValue) -> Self {
    self.file.cache_control = Some(value);
    self
  }

  /// Finish configuration.
  pub fn build(self) -> ServeFile {
    self.file
  }
}

impl ServeFile {
  /// Configure a single file responder.
  pub fn builder(path: impl Into<PathBuf>) -> ServeFileBuilder {
    ServeFileBuilder::new(path)
  }

  /// Serve the configured file for GET/HEAD, irrespective of the request URI.
  ///
  /// Mount on a specific route; use [`ServeDir`](super::ServeDir) for path-aware serving.
  pub async fn handle(&self, request: Request) -> Response {
    let (parts, _) = request.into_parts();
    if let Some(response) = response::method_error(&parts) {
      return response;
    }
    let Ok(file) = OpenFile::open(&self.path).await else {
      return StatusCode::NOT_FOUND.into_response();
    };
    response::serve(
      file,
      &self.path,
      None,
      false,
      self.cache_control.as_ref(),
      &parts,
    )
    .await
    .unwrap_or_else(|error| {
      tracing::debug!(%error, "could not stream static file");
      StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })
  }
}
