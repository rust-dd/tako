use std::io::Write;

use flate2::Compression as GzLevel;
use flate2::write::GzEncoder;
use flate2::write::ZlibEncoder;
#[cfg(feature = "zstd")]
use zstd::stream::encode_all as zstd_encode;

pub(crate) fn compress_gzip(data: &[u8], lvl: u32) -> std::io::Result<Vec<u8>> {
  let mut enc = GzEncoder::new(Vec::new(), GzLevel::new(lvl));
  enc.write_all(data)?;
  enc.finish()
}

pub(crate) fn compress_brotli(data: &[u8], lvl: u32) -> std::io::Result<Vec<u8>> {
  // One write with a size hint: fed in small chunks, the encoder allocates its whole window.
  let params = brotli::enc::BrotliEncoderParams {
    quality: lvl as i32,
    lgwin: 22,
    size_hint: data.len(),
    ..Default::default()
  };
  let mut encoder =
    brotli::CompressorWriter::with_params(Vec::new(), data.len().max(4096), &params);
  encoder.write_all(data)?;
  Ok(encoder.into_inner())
}

pub(crate) fn compress_deflate(data: &[u8], lvl: u32) -> std::io::Result<Vec<u8>> {
  let mut enc = ZlibEncoder::new(Vec::new(), flate2::Compression::new(lvl));
  enc.write_all(data)?;
  enc.finish()
}

#[cfg(feature = "zstd")]
pub(crate) fn compress_zstd(data: &[u8], lvl: i32) -> std::io::Result<Vec<u8>> {
  zstd_encode(data, lvl)
}
