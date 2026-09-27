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

#[cfg(not(feature = "compio"))]
#[test]
fn buffered_compression_yields_while_the_blocking_worker_is_busy() {
  use std::time::Duration;

  use tako_rs_core::router::Router;

  tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .max_blocking_threads(1)
    .build()
    .unwrap()
    .block_on(async {
      let (release, blocked) = std::sync::mpsc::channel();
      let (ready, started) = tokio::sync::oneshot::channel();
      let occupied = tokio::task::spawn_blocking(move || {
        ready.send(()).unwrap();
        blocked.recv_timeout(Duration::from_secs(5)).unwrap();
      });
      started.await.unwrap();
      let original = Bytes::from(vec![b'x'; 128 * 1024]);
      let body = original.clone();
      let mut router = Router::new();
      router.plugin(
        super::builder::CompressionBuilder::new()
          .content_types(super::config::ContentTypePolicy::Exact(vec![
            "application/octet-stream".into(),
          ]))
          .build(),
      );
      router.get("/", move || {
        let body = body.clone();
        async move { body }
      });
      let request = http::Request::builder()
        .header("accept-encoding", "gzip")
        .body(TakoBody::empty())
        .unwrap();
      let response = router.dispatch(request);
      let mut response = std::pin::pin!(response);
      assert!(futures_util::poll!(response.as_mut()).is_pending());
      release.send(()).unwrap();
      let response = tokio::time::timeout(Duration::from_secs(2), response)
        .await
        .unwrap();
      assert_eq!(response.headers()["content-encoding"], "gzip");
      let encoded = response.into_body().collect().await.unwrap().to_bytes();
      assert_eq!(
        decode(flate2::read::GzDecoder::new(encoded.as_ref())),
        original
      );
      occupied.await.unwrap();
    });
}

#[cfg(feature = "compio")]
#[test]
fn compio_buffered_compression_round_trips_an_offloaded_body() {
  compio::runtime::Runtime::new().unwrap().block_on(async {
    let original = "buffered response ".repeat(8192);
    let body = original.clone();
    let mut router = tako_rs_core::router::Router::new();
    router.plugin(super::builder::CompressionBuilder::new().build());
    router.get("/", move || {
      let body = body.clone();
      async move { body }
    });
    let request = http::Request::builder()
      .header("accept-encoding", "gzip")
      .body(TakoBody::empty())
      .unwrap();
    let response = router.dispatch(request).await;
    assert_eq!(response.headers()["content-encoding"], "gzip");
    let encoded = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
      decode(flate2::read::GzDecoder::new(encoded.as_ref())),
      original.as_bytes()
    );
  });
}
