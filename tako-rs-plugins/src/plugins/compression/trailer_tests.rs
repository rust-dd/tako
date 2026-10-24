use std::io::Read;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use bytes::Bytes;
use futures_util::StreamExt;
use http::HeaderMap;
use http_body::Body;
use http_body::Frame;
use http_body::SizeHint;
use http_body_util::BodyExt;
use tako_rs_core::body::TakoBody;
use tako_rs_core::types::BoxError;

type Codec = (&'static str, fn(TakoBody) -> TakoBody, fn(&[u8]) -> Vec<u8>);

fn decode(mut reader: impl Read) -> Vec<u8> {
  let mut output = Vec::new();
  reader.read_to_end(&mut output).unwrap();
  output
}

fn codecs() -> Vec<Codec> {
  vec![
    (
      "gzip",
      |body| super::gzip_stream::stream_gzip(body, 3),
      |data| decode(flate2::read::GzDecoder::new(data)),
    ),
    (
      "br",
      |body| super::brotli_stream::stream_brotli(body, 3),
      |data| decode(brotli::Decompressor::new(data, 4096)),
    ),
    (
      "deflate",
      |body| super::deflate_stream::stream_deflate(body, 3),
      |data| decode(flate2::read::ZlibDecoder::new(data)),
    ),
    #[cfg(feature = "zstd")]
    (
      "zstd",
      |body| super::zstd_stream::stream_zstd(body, 3),
      |data| zstd::stream::decode_all(data).unwrap(),
    ),
  ]
}

fn trailers() -> HeaderMap {
  let mut headers = HeaderMap::new();
  headers.insert("x-final-state", "complete".parse().unwrap());
  headers.append("x-value", "one".parse().unwrap());
  headers.append("x-value", "two".parse().unwrap());
  headers
}

fn frames(fail: bool) -> Vec<Result<Frame<Bytes>, BoxError>> {
  let mut frames = vec![Ok(Frame::data(Bytes::from_static(b"first")))];
  if fail {
    frames.push(Err(std::io::Error::other("source failed").into()));
  }
  frames.extend([
    Ok(Frame::data(Bytes::from_static(b" second"))),
    Ok(Frame::trailers(trailers())),
  ]);
  frames
}

#[tokio::test]
async fn every_codec_preserves_trailers_after_its_complete_output() {
  for (name, compress, decode) in codecs() {
    let input = futures_util::stream::iter(frames(false)).then(|frame| async {
      tokio::task::yield_now().await;
      frame
    });
    let mut output = compress(TakoBody::from_try_stream(input));
    let mut data = Vec::new();
    let mut received_trailers = None;
    while let Some(frame) = output.frame().await {
      match frame.unwrap().into_data() {
        Ok(chunk) => {
          assert!(received_trailers.is_none(), "{name}: data after trailers");
          data.extend_from_slice(&chunk);
        }
        Err(frame) => received_trailers = Some(frame.into_trailers().unwrap()),
      }
    }
    assert_eq!(received_trailers.as_ref(), Some(&trailers()), "{name}");
    assert_eq!(decode(&data), b"first second", "{name}");
  }
}

#[tokio::test]
async fn every_codec_stops_after_a_source_error() {
  for (name, compress, _) in codecs() {
    let body = TakoBody::from_try_stream(futures_util::stream::iter(frames(true)));
    let mut output = compress(body);
    loop {
      let frame = output
        .frame()
        .await
        .expect("source error must be propagated");
      if let Err(error) = frame {
        assert_eq!(error.to_string(), "source failed", "{name}");
        break;
      }
    }
    assert!(
      output.frame().await.is_none(),
      "{name}: data or trailers after error"
    );
  }
}

struct SizedFrames(std::vec::IntoIter<Result<Frame<Bytes>, BoxError>>);

impl Body for SizedFrames {
  type Data = Bytes;
  type Error = BoxError;

  fn poll_frame(
    mut self: Pin<&mut Self>,
    _: &mut Context<'_>,
  ) -> Poll<Option<Result<Frame<Bytes>, BoxError>>> {
    Poll::Ready(self.0.next())
  }

  fn size_hint(&self) -> SizeHint {
    SizeHint::with_exact(12)
  }
}

#[tokio::test]
async fn buffered_compression_preserves_trailers_and_propagates_errors() {
  for fail in [false, true] {
    let mut router = tako_rs_core::router::Router::new();
    router.plugin(super::CompressionBuilder::new().min_size(0).build());
    router.get("/", move || async move {
      TakoBody::new(SizedFrames(frames(fail).into_iter()))
    });
    let response = router
      .dispatch(
        http::Request::builder()
          .header("accept-encoding", "gzip")
          .body(TakoBody::empty())
          .unwrap(),
      )
      .await;
    let collected = response.into_body().collect().await;
    if fail {
      assert!(
        collected.is_err(),
        "buffering must not turn a source error into success"
      );
    } else {
      let collected = collected.unwrap();
      assert_eq!(collected.trailers(), Some(&trailers()));
      assert_eq!(
        decode(flate2::read::GzDecoder::new(collected.to_bytes().as_ref())),
        b"first second"
      );
    }
  }
}
