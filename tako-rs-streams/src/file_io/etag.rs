//! Weak validators derived from file metadata.

use std::time::SystemTime;
use std::time::UNIX_EPOCH;

/// Derive a quoted weak `ETag` from the size and nanosecond modification time.
///
/// Metadata cannot prove byte-for-byte equality when timestamps are preserved;
/// use a content hash when strong validation is required.
pub fn weak_etag_from_metadata(size: u64, mtime: SystemTime) -> String {
  let timestamp = mtime
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_nanos();
  format!("W/\"{size:x}-{timestamp:x}\"")
}
