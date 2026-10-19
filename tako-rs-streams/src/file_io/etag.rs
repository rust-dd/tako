use std::time::SystemTime;
use std::time::UNIX_EPOCH;

/// A quoted weak `ETag` from size and nanosecond mtime; a copy that keeps timestamps gets the
/// same tag, so use a content hash when a strong validator is needed.
pub fn weak_etag_from_metadata(size: u64, mtime: SystemTime) -> String {
  let timestamp = mtime
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_nanos();
  format!("W/\"{size:x}-{timestamp:x}\"")
}
