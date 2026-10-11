use std::path::PathBuf;

use http::HeaderMap;
use http::StatusCode;
use http::header;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

use super::dir::ServeDir;
use crate::file_io::OpenFile;
use crate::file_io::response;

struct Resolved {
  original: PathBuf,
  compressed: Option<(PathBuf, &'static str)>,
}

struct Opened {
  file: OpenFile,
  original: PathBuf,
  encoding: Option<&'static str>,
}

impl Resolved {
  #[cfg(not(feature = "compio"))]
  fn open_blocking(self) -> Option<Opened> {
    if let Some((path, encoding)) = &self.compressed
      && let Ok(file) = OpenFile::open_blocking(path)
    {
      return Some(Opened {
        file,
        original: self.original,
        encoding: Some(encoding),
      });
    }
    let file = OpenFile::open_blocking(&self.original).ok()?;
    Some(Opened {
      file,
      original: self.original,
      encoding: None,
    })
  }

  #[cfg(feature = "compio")]
  async fn open(self) -> Option<Opened> {
    if let Some((path, encoding)) = &self.compressed
      && let Ok(file) = OpenFile::open(path).await
    {
      return Some(Opened {
        file,
        original: self.original,
        encoding: Some(encoding),
      });
    }
    let file = OpenFile::open(&self.original).await.ok()?;
    Some(Opened {
      file,
      original: self.original,
      encoding: None,
    })
  }
}

impl ServeDir {
  /// Serve a GET or HEAD request, decoding the URL before checking path components.
  pub async fn handle(&self, request: Request) -> Response {
    let (parts, _) = request.into_parts();
    if let Some(response) = response::method_error(&parts) {
      return response;
    }
    let Ok(decoded) = percent_encoding::percent_decode_str(parts.uri.path()).decode_utf8() else {
      return StatusCode::NOT_FOUND.into_response();
    };
    let relative = decoded.trim_start_matches('/');
    if !relative.is_empty() && !self.config.valid_relative(relative) {
      return StatusCode::NOT_FOUND.into_response();
    }
    let mut encodings = [
      ("br", self.config.precompressed.brotli),
      ("gzip", self.config.precompressed.gzip),
    ]
    .map(|(name, enabled)| {
      (
        name,
        if enabled {
          quality(&parts.headers, name)
        } else {
          0.0
        },
      )
    });
    if encodings[1].1 > encodings[0].1 {
      encodings.swap(0, 1);
    }
    let config = self.config.clone();
    let relative = relative.to_owned();
    let resolve = move || {
      let base = config.base.canonicalize().ok()?;
      let target = config
        .within_base(&base.join(relative), &base)
        .and_then(|path| {
          if path.is_dir() {
            config
              .index_files
              .iter()
              .filter(|index| config.valid_relative(index))
              .find_map(|index| {
                let path = config.within_base(&path.join(index), &base)?;
                path.is_file().then_some(path)
              })
          } else {
            path.is_file().then_some(path)
          }
        })
        .or_else(|| config.fallback.clone().filter(|path| path.is_file()))?;
      let compressed = encodings
        .into_iter()
        .filter(|(_, q)| *q > 0.0)
        .find_map(|(encoding, _)| {
          let mut sidecar = target.as_os_str().to_owned();
          sidecar.push(if encoding == "br" { ".br" } else { ".gz" });
          let path = config.within_base(&PathBuf::from(sidecar), &base)?;
          path.is_file().then_some((path, encoding))
        });
      Some(Resolved {
        original: target,
        compressed,
      })
    };
    #[cfg(not(feature = "compio"))]
    let opened = tokio::task::spawn_blocking(move || resolve().and_then(Resolved::open_blocking))
      .await
      .ok()
      .flatten();
    #[cfg(feature = "compio")]
    let opened = match send_wrapper::SendWrapper::new(compio::runtime::spawn_blocking(resolve))
      .await
      .ok()
      .flatten()
    {
      Some(resolved) => resolved.open().await,
      None => None,
    };
    let Some(opened) = opened else {
      return StatusCode::NOT_FOUND.into_response();
    };
    response::serve(
      opened.file,
      &opened.original,
      opened.encoding,
      self.config.precompressed.brotli || self.config.precompressed.gzip,
      self.config.cache_control.as_ref(),
      &parts,
    )
    .await
    .unwrap_or_else(|error| {
      tracing::debug!(%error, "could not stream static file");
      StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })
  }
}

fn quality(headers: &HeaderMap, encoding: &str) -> f32 {
  let mut wildcard = 0.0;
  let mut explicit = None;
  for value in headers
    .get_all(header::ACCEPT_ENCODING)
    .iter()
    .filter_map(|value| value.to_str().ok())
  {
    for entry in value.split(',') {
      let mut parts = entry.split(';');
      let name = parts.next().unwrap_or("").trim();
      let mut quality = 1.0;
      for parameter in parts {
        if let Some((name, value)) = parameter.trim().split_once('=')
          && name.eq_ignore_ascii_case("q")
        {
          quality = value
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|q| (0.0..=1.0).contains(q))
            .unwrap_or(0.0);
        }
      }
      if name.eq_ignore_ascii_case(encoding) {
        explicit = Some(quality);
      } else if name == "*" {
        wildcard = quality;
      }
    }
  }
  explicit.unwrap_or(wildcard)
}
