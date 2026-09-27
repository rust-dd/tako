use std::io::Read;

use bytes::Bytes;
use http_body_util::BodyExt;
use tako_rs_core::body::TakoBody;

use super::brotli_stream::stream_brotli;
use super::deflate_stream::stream_deflate;
use super::encoder::compress_deflate;
use super::gzip_stream::stream_gzip;
#[cfg(feature = "zstd")]
use super::zstd_stream::stream_zstd;

fn input() -> TakoBody {
  TakoBody::from_stream(futures_util::stream::iter([
    Ok::<_, std::io::Error>(Bytes::from_static(b"first chunk")),
    Ok(Bytes::new()),
    Ok(Bytes::from_static(b" and second chunk")),
  ]))
}

fn decode(mut reader: impl Read) -> Vec<u8> {
  let mut decoded = Vec::new();
  reader.read_to_end(&mut decoded).unwrap();
  decoded
}

#[tokio::test]
async fn compression_streams_round_trip_multiple_chunks_and_finish() {
  let expected = b"first chunk and second chunk";
  let gzip = stream_gzip(input(), 5).collect().await.unwrap().to_bytes();
  assert_eq!(
    decode(flate2::read::GzDecoder::new(gzip.as_ref())),
    expected
  );

  let deflate = stream_deflate(input(), 5)
    .collect()
    .await
    .unwrap()
    .to_bytes();
  assert_eq!(
    decode(flate2::read::ZlibDecoder::new(deflate.as_ref())),
    expected
  );

  let brotli = stream_brotli(input(), 5)
    .collect()
    .await
    .unwrap()
    .to_bytes();
  assert_eq!(
    decode(brotli::Decompressor::new(brotli.as_ref(), 4096)),
    expected
  );

  #[cfg(feature = "zstd")]
  {
    let zstd = stream_zstd(input(), 3).collect().await.unwrap().to_bytes();
    assert_eq!(zstd::stream::decode_all(zstd.as_ref()).unwrap(), expected);
  }
}

#[test]
fn buffered_http_deflate_uses_a_zlib_wrapper() {
  let input = b"a deflate-encoded HTTP response";
  let encoded = compress_deflate(input, 5).unwrap();
  assert_eq!(
    decode(flate2::read::ZlibDecoder::new(encoded.as_slice())),
    input
  );
}
