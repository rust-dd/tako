use std::path::Path;
use std::path::PathBuf;

#[cfg(feature = "compio")]
use compio::fs;
use http::StatusCode;
use http::header;
use tako_rs_core::body::TakoBody;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;
#[cfg(not(feature = "compio"))]
use tokio::fs;
#[cfg(not(feature = "compio"))]
use tokio::io::AsyncReadExt;

use super::dir::ServeDir;

impl ServeDir {
  async fn resolve_existing(
    &self,
    file_path: PathBuf,
    headers: &http::HeaderMap,
  ) -> Option<(Response, &'static str)> {
    // Index resolution if pointing at a directory.
    let target = if file_path.is_dir() {
      let mut chosen: Option<PathBuf> = None;
      for idx in &self.index_files {
        let cand = file_path.join(idx);
        if !cand.is_file() {
          continue;
        }
        // Critical: re-run the canonical-within-base check on the index
        // file. `cand.is_file()` follows symlinks, so an in-base
        // `index.html` that links out of the base (e.g. to `/etc/passwd`)
        // would otherwise be served — exactly the escape vector the
        // sidecar fix (C8) closed on the `.br`/`.gz` path. Cold path —
        // only fires when the request targets a directory.
        if let Some(canonical) = self.canonical_within_base(&cand) {
          chosen = Some(canonical);
          break;
        }
      }
      chosen?
    } else {
      file_path
    };

    if let Some((compressed, encoding)) = self.precompressed_variant(&target, headers) {
      if let Some(resp) = Self::serve_file_with_encoding(&compressed, &target, encoding).await {
        return Some((resp, encoding));
      }
      // Sidecar read failed (deleted between resolve and open, permission
      // glitch, etc.) — fall through to the identity file instead of
      // 404-ing the whole request.
      tracing::debug!(
        target = %target.display(),
        encoding,
        "precompressed sidecar read failed, falling back to identity"
      );
    }

    Some((Self::serve_file(&target).await?, "identity"))
  }

  /// Open the file via a single `File::open` (resolves symlinks exactly once),
  /// verify the result is a regular file via the open FD's metadata (defense
  /// in depth against directory/special-file confusion), then read. This
  /// replaces the prior `fs::read` pattern which would re-resolve the path
  /// after the caller had already canonicalized it.
  #[cfg(not(feature = "compio"))]
  async fn open_and_read_regular(path: &Path) -> Option<Vec<u8>> {
    let mut file = fs::File::open(path).await.ok()?;
    let meta = file.metadata().await.ok()?;
    if !meta.is_file() {
      return None;
    }
    let mut contents = Vec::with_capacity(meta.len() as usize);
    file.read_to_end(&mut contents).await.ok()?;
    Some(contents)
  }

  #[cfg(feature = "compio")]
  async fn open_and_read_regular(path: &Path) -> Option<Vec<u8>> {
    // compio uses positional read with owned buffers; the high-level
    // `fs::read` already wraps open + read + metadata. The canonical-prefix
    // check is performed by the caller before we get here.
    //
    // STR-8: this loads the whole file into RAM, unlike the tokio path
    // that streams via ReaderStream. Operators serving large static
    // assets under compio must cap file sizes at the route level (or
    // switch to the tokio backend) — a multi-GB asset will land as a
    // single `Vec<u8>` per request. Replacing with a chunked
    // `compio::fs::File::read_at` loop is a 2.x deferral; the type
    // surface change ripples through `StaticDir`'s body shape.
    let meta = fs::metadata(path).await.ok()?;
    if !meta.is_file() {
      return None;
    }
    fs::read(path).await.ok()
  }

  async fn serve_file(file_path: &Path) -> Option<Response> {
    let contents = Self::open_and_read_regular(file_path).await?;
    let mime = mime_guess::from_path(file_path).first_or_octet_stream();
    Some(
      http::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.to_string())
        .body(TakoBody::from(contents))
        .unwrap(),
    )
  }

  async fn serve_file_with_encoding(
    compressed: &Path,
    original: &Path,
    encoding: &'static str,
  ) -> Option<Response> {
    let contents = Self::open_and_read_regular(compressed).await?;
    let mime = mime_guess::from_path(original).first_or_octet_stream();
    Some(
      http::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.to_string())
        .header(header::CONTENT_ENCODING, encoding)
        .header(header::VARY, "Accept-Encoding")
        .body(TakoBody::from(contents))
        .unwrap(),
    )
  }

  /// Handles an HTTP request to serve a static file from the directory.
  pub async fn handle(&self, req: Request) -> impl Responder {
    let path = req.uri().path();
    let headers = req.headers().clone();

    if let Some(file_path) = self.sanitize_path(path)
      && let Some((resp, _enc)) = self.resolve_existing(file_path, &headers).await
    {
      return resp;
    }

    if let Some(fallback) = &self.fallback
      && let Some((resp, _)) = self.resolve_existing(fallback.clone(), &headers).await
    {
      return resp;
    }

    http::Response::builder()
      .status(StatusCode::NOT_FOUND)
      .body(TakoBody::from("File not found"))
      .unwrap()
  }
}
