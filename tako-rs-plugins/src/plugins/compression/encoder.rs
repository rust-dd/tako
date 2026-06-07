//! Buffered (whole-body) compression encoders for each supported algorithm.

use std::io::Read;
use std::io::Write;

use flate2::Compression as GzLevel;
use flate2::write::DeflateEncoder;
use flate2::write::GzEncoder;
#[cfg(feature = "zstd")]
use zstd::stream::encode_all as zstd_encode;

/// Compresses data using Gzip algorithm.
pub(crate) fn compress_gzip(data: &[u8], lvl: u32) -> std::io::Result<Vec<u8>> {
  let mut enc = GzEncoder::new(Vec::new(), GzLevel::new(lvl));
  enc.write_all(data)?;
  enc.finish()
}

/// Compresses data using Brotli algorithm.
pub(crate) fn compress_brotli(data: &[u8], lvl: u32) -> std::io::Result<Vec<u8>> {
  let mut out = Vec::new();
  brotli::CompressorReader::new(data, 4096, lvl, 22)
    .read_to_end(&mut out)
    .map_err(|_| std::io::Error::other("Failed to compress data"))?;
  Ok(out)
}

/// Compresses data using DEFLATE algorithm.
pub(crate) fn compress_deflate(data: &[u8], lvl: u32) -> std::io::Result<Vec<u8>> {
  let mut enc = DeflateEncoder::new(Vec::new(), flate2::Compression::new(lvl));
  enc.write_all(data)?;
  enc.finish()
}

/// Compresses data using Zstandard algorithm (requires zstd feature).
#[cfg(feature = "zstd")]
#[cfg_attr(docsrs, doc(cfg(feature = "zstd")))]
pub(crate) fn compress_zstd(data: &[u8], lvl: i32) -> std::io::Result<Vec<u8>> {
  zstd_encode(data, lvl)
}
