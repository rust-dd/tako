//! Weak `ETag` derivation from coarse file metadata.

use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use sha1::Digest as _;
use sha1::Sha1;

/// Helper that hashes (size + mtime) into a **weak** `ETag` (`W/"…"`).
///
/// SHA-1 over coarse metadata cannot prove byte-for-byte equivalence — two
/// files written within the same wall-clock second with the same size will
/// hash to the same digest. Returning the value pre-wrapped in `W/"…"` keeps
/// downstream callers honest about that limitation: clients (and caches)
/// won't assume strong validation semantics. Callers should pass the value
/// straight to `Response.header(ETAG, …)` without re-quoting.
pub fn weak_etag_from_metadata(size: u64, mtime: SystemTime) -> String {
  let mtime_secs = mtime.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
  let mut hasher = Sha1::new();
  hasher.update(size.to_le_bytes());
  hasher.update(mtime_secs.to_le_bytes());
  let digest = hasher.finalize();
  let mut out = String::with_capacity(44);
  out.push_str("W/\"");
  for b in digest {
    out.push_str(&format!("{b:02x}"));
  }
  out.push('"');
  out
}
