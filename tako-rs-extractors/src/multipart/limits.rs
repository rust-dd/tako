use std::sync::Arc;

use multer::Constraints;
use multer::SizeLimit;

/// Per-route or global configuration for multipart extraction.
///
/// Insert into request extensions (or set as global state) to constrain how
/// `TakoMultipart` / `TakoTypedMultipart` consume request bodies. Defaults
/// are permissive — opt in to limits explicitly.
#[derive(Debug, Clone)]
pub struct MultipartConfig {
  /// Total request body cap, in bytes. `None` = no whole-payload limit.
  pub total_size_limit: Option<u64>,
  /// Per-part body cap, in bytes. `None` = no per-part limit.
  ///
  /// Defaults to 1 MiB to keep text-field deserialization (`from_utf8` on a
  /// fully-collected `Vec<u8>`) bounded; raise it explicitly when handling
  /// genuinely large fields.
  pub per_part_size_limit: Option<u64>,
  /// Maximum number of parts. Reaching this returns an error mid-parse.
  ///
  /// Enforced by [`TakoTypedMultipart`](crate::multipart::TakoTypedMultipart); the raw [`TakoMultipart`](crate::multipart::TakoMultipart) does not
  /// enforce this because users may consume the inner `multer::Multipart`
  /// directly. Prefer the typed extractor when you need the cap.
  pub max_parts: Option<usize>,
  /// Allow-list of part content-types (e.g. `image/png`, `application/pdf`).
  /// `None` (or empty) = accept any.
  pub allowed_content_types: Option<Arc<Vec<String>>>,
  /// When uploading via `UploadedFile`, switch from in-memory buffering to a
  /// temp file once the part exceeds this many bytes. `None` = always disk.
  pub disk_spill_threshold: Option<u64>,
  /// Maximum time to read a whole multipart field before aborting the
  /// request. Despite the historical "chunk" naming, the timeout currently
  /// wraps the *whole-field* read future ([`TakoTypedMultipart`](crate::multipart::TakoTypedMultipart)'s
  /// `field.bytes().await`) — it bounds total per-field wall-clock, not
  /// inter-chunk gaps. A slow-drip client whose total payload arrives
  /// within this window still passes; tune for the slowest realistic
  /// full-field upload, not per-chunk latency.
  ///
  /// `None` disables the timeout entirely. Per-chunk semantics (re-arming
  /// on each frame) are tracked for a 2.x revision.
  pub field_chunk_timeout: Option<std::time::Duration>,
}

impl Default for MultipartConfig {
  fn default() -> Self {
    Self {
      total_size_limit: None,
      // Bound the per-part allocation by default so an unconfigured
      // application doesn't OOM on a hostile multipart upload.
      per_part_size_limit: Some(1024 * 1024),
      max_parts: None,
      allowed_content_types: None,
      disk_spill_threshold: None,
      field_chunk_timeout: None,
    }
  }
}

impl MultipartConfig {
  /// Build a permissive config (no limits). Configure via the builder methods.
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

  /// Maximum time to wait for a single chunk from any multipart field. See
  /// [`Self::field_chunk_timeout`].
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

  /// Set the in-memory → disk spill threshold for `UploadedFile`.
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
    // EXT-7: bare `starts_with` would admit `image/pngx` against an
    // allowlist of `image/png` (false-positive on prefix). Accept only
    // when the content-type *equals* the allow entry, or when the entry
    // is followed by an RFC 7231 §3.1.1.1 parameter delimiter (`;`) or
    // whitespace — those are the only legal continuations.
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
