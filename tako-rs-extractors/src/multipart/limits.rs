use std::sync::Arc;

use multer::Constraints;
use multer::SizeLimit;

/// Multipart limits, read from a request extension, then `Router::with_state`, then a
/// process-global value registered with `get_or_init_state`.
#[derive(Debug, Clone)]
pub struct MultipartConfig {
  /// Whole-request body cap in bytes. Typed extraction also applies `BodyLimit`, using
  /// the smaller cap; `None` leaves only that shared cap for typed extraction.
  pub total_size_limit: Option<u64>,
  /// Per-part body cap in bytes; the 1 MiB default bounds text fields, which are buffered whole.
  pub per_part_size_limit: Option<u64>,
  /// Maximum number of parts, enforced by both multipart extractors.
  pub max_parts: Option<usize>,
  /// Allowed part content types, such as `image/png`; `None` or empty allows any.
  /// Enforced by both multipart extractors.
  pub allowed_content_types: Option<Arc<Vec<String>>>,
  /// Size above which `BufferedUploadedFile` spills to disk; `None` keeps uploads in memory.
  /// The typed extractor passes its resolved config to the file reader.
  pub disk_spill_threshold: Option<u64>,
  /// Wall-clock limit for reading each whole field, not each chunk despite the name.
  /// Only `TakoTypedMultipart` enforces it.
  pub field_chunk_timeout: Option<std::time::Duration>,
}

impl Default for MultipartConfig {
  fn default() -> Self {
    Self {
      total_size_limit: None,
      // Bounds per-part allocation so an unconfigured app cannot be exhausted by one upload.
      per_part_size_limit: Some(1024 * 1024),
      max_parts: None,
      allowed_content_types: None,
      disk_spill_threshold: None,
      field_chunk_timeout: None,
    }
  }
}

impl MultipartConfig {
  /// Same as `Default`: only the 1 MiB per-part cap is set.
  pub fn new() -> Self {
    Self::default()
  }

  /// Set a whole-request body cap.
  pub fn total_size_limit(mut self, bytes: u64) -> Self {
    self.total_size_limit = Some(bytes);
    self
  }

  /// Set a per-part body cap.
  pub fn per_part_size_limit(mut self, bytes: u64) -> Self {
    self.per_part_size_limit = Some(bytes);
    self
  }

  /// Set the maximum number of parts.
  pub fn max_parts(mut self, n: usize) -> Self {
    self.max_parts = Some(n);
    self
  }

  /// Set the time limit for reading each whole field.
  pub fn field_chunk_timeout(mut self, d: std::time::Duration) -> Self {
    self.field_chunk_timeout = Some(d);
    self
  }

  /// Replace the allow-list of accepted part content-types.
  pub fn allowed_content_types<I, S>(mut self, types: I) -> Self
  where
    I: IntoIterator<Item = S>,
    S: Into<String>,
  {
    self.allowed_content_types = Some(Arc::new(types.into_iter().map(Into::into).collect()));
    self
  }

  /// Set the size above which `BufferedUploadedFile` spills to disk.
  pub fn disk_spill_threshold(mut self, bytes: u64) -> Self {
    self.disk_spill_threshold = Some(bytes);
    self
  }

  pub(crate) fn to_constraints(&self) -> Constraints {
    let mut limit = SizeLimit::new();
    if let Some(b) = self.total_size_limit {
      limit = limit.whole_stream(b);
    }
    if let Some(b) = self.per_part_size_limit {
      limit = limit.per_field(b);
    }
    Constraints::new().size_limit(limit)
  }

  pub(crate) fn lookup(req_ext: &http::Extensions) -> MultipartConfig {
    if let Some(cfg) = req_ext.get::<MultipartConfig>() {
      return cfg.clone();
    }
    if let Some(rs) = req_ext.get::<Arc<tako_rs_core::router_state::RouterState>>()
      && let Some(arc) = rs.get::<MultipartConfig>()
    {
      return (*arc).clone();
    }
    if let Some(arc) = tako_rs_core::state::get_state::<MultipartConfig>() {
      return arc.as_ref().clone();
    }
    MultipartConfig::default()
  }

  pub(crate) fn content_type_ok(&self, ct: Option<&str>) -> bool {
    let Some(allow) = self.allowed_content_types.as_ref() else {
      return true;
    };
    if allow.is_empty() {
      return true;
    }
    let ct = ct.unwrap_or("");
    // Exact match, or the entry followed by `;` or whitespace (RFC 7231 parameters),
    // so an allowed `image/png` does not admit `image/pngx`.
    allow.iter().any(|a| {
      let a = a.as_str();
      ct == a
        || ct.strip_prefix(a).is_some_and(|rest| {
          rest
            .chars()
            .next()
            .is_some_and(|c| c == ';' || c == ' ' || c == '\t')
        })
    })
  }
}
