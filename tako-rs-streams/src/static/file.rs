use std::path::PathBuf;

#[cfg(feature = "compio")]
use compio::fs;
use http::StatusCode;
use tako_rs_core::body::TakoBody;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;
#[cfg(not(feature = "compio"))]
use tokio::fs;

/// Static file server for serving individual files.
#[doc(alias = "serve_file")]
pub struct ServeFile {
  path: PathBuf,
}

/// Builder for configuring a `ServeFile` instance.
#[must_use]
pub struct ServeFileBuilder {
  path: PathBuf,
}

impl ServeFileBuilder {
  /// Creates a new builder with the specified file path.
  #[inline]
  pub fn new<P: Into<PathBuf>>(path: P) -> Self {
    Self { path: path.into() }
  }

  /// Builds and returns the configured `ServeFile` instance.
  #[inline]
  #[must_use]
  pub fn build(self) -> ServeFile {
    ServeFile { path: self.path }
  }
}

impl ServeFile {
  /// Creates a new builder for configuring a `ServeFile`.
  pub fn builder<P: Into<PathBuf>>(path: P) -> ServeFileBuilder {
    ServeFileBuilder::new(path)
  }

  /// Serves the configured file with appropriate MIME type.
  async fn serve_file(&self) -> Option<Response> {
    match fs::read(&self.path).await {
      Ok(contents) => {
        let mime = mime_guess::from_path(&self.path).first_or_octet_stream();
        Some(
          http::Response::builder()
            .status(StatusCode::OK)
            .header(http::header::CONTENT_TYPE, mime.to_string())
            .body(TakoBody::from(contents))
            .unwrap(),
        )
      }
      Err(_) => None,
    }
  }

  /// Handles an HTTP request to serve the configured static file.
  ///
  /// The request itself is **ignored** — `ServeFile` always serves the file
  /// configured on the builder, regardless of `req.uri()`. Mount this
  /// handler on a single specific route (e.g. `/manifest.json`), not on a
  /// catch-all glob, otherwise every URL under that glob will return the
  /// same file. Use [`ServeDir`](super::ServeDir) when you want path-aware static serving.
  pub async fn handle(&self, _req: Request) -> impl Responder {
    if let Some(resp) = self.serve_file().await {
      resp
    } else {
      let mut resp = http::Response::new(TakoBody::from("File not found"));
      *resp.status_mut() = StatusCode::NOT_FOUND;
      resp
    }
  }
}
