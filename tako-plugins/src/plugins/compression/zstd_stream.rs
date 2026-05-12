#![cfg(feature = "zstd")]

//! Zstandard compression streaming utilities for high-performance HTTP response compression.
//!
//! This module provides streaming Zstandard (zstd) compression for HTTP response bodies,
//! offering excellent compression ratios with fast decompression speeds. Zstandard is
//! particularly well-suited for modern web applications that require both high compression
//! efficiency and low latency. The streaming implementation enables memory-efficient
//! compression of large responses without buffering entire content in memory.
//!
//! # Examples
//!
//! ```rust
//! # #[cfg(feature = "zstd")]
//! use tako::plugins::compression::zstd_stream::stream_zstd;
//! # #[cfg(feature = "zstd")]
//! use http_body_util::Full;
//! # #[cfg(feature = "zstd")]
//! use bytes::Bytes;
//!
//! # #[cfg(feature = "zstd")]
//! # fn example() {
//! // Compress a response body with Zstandard level 3
//! let body = Full::from(Bytes::from("Hello, World! This is test content."));
//! let compressed = stream_zstd(body, 3);
//!
//! // High compression for static assets
//! let static_content = Full::from(Bytes::from("Large static file content..."));
//! let high_compressed = stream_zstd(static_content, 19);
//! # }
//! ```

use std::io::Write;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use bytes::Bytes;
use futures_util::Stream;
use futures_util::TryStreamExt;
use http_body::Body;
use http_body::Frame;
use http_body_util::BodyExt;
use pin_project_lite::pin_project;
use tako_core::body::TakoBody;
use tako_core::types::BoxError;
use zstd::stream::Encoder;

/// Compresses an HTTP body stream using Zstandard compression algorithm.
pub fn stream_zstd<B>(body: B, level: i32) -> TakoBody
where
  B: Body<Data = Bytes, Error = BoxError> + Send + 'static,
{
  let upstream = body.into_data_stream();
  let zstd_stream = ZstdStream::new(upstream, level).map_ok(Frame::data);
  TakoBody::from_try_stream(zstd_stream)
}

pin_project! {
    /// Streaming Zstandard compressor that wraps an inner data stream.
    pub struct ZstdStream<S> {
        #[pin] inner: S,
        encoder: Option<Encoder<'static, Vec<u8>>>,
        // Bytes already produced by `encoder.finish()` once the upstream
        // closed — held separately because the encoder is consumed at that
        // point.
        tail: Vec<u8>,
        done: bool,
    }
}

impl<S> ZstdStream<S> {
  /// Creates a new Zstandard compression stream with the specified compression level.
  fn new(stream: S, level: i32) -> Self {
    Self {
      inner: stream,
      encoder: Some(Encoder::new(Vec::new(), level).expect("zstd encoder")),
      tail: Vec::new(),
      done: false,
    }
  }
}

impl<S> Stream for ZstdStream<S>
where
  S: Stream<Item = Result<Bytes, BoxError>>,
{
  type Item = Result<Bytes, BoxError>;

  /// Polls the stream for the next compressed data chunk.
  fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    let mut this = self.project();

    loop {
      // 1) Drain the encoder's internal buffer (live) or the tail (post-finish)
      //    rather than copying out and then growing the original Vec forever.
      if let Some(enc) = this.encoder.as_mut() {
        if !enc.get_ref().is_empty() {
          let chunk: Vec<u8> = enc.get_mut().drain(..).collect();
          return Poll::Ready(Some(Ok(Bytes::from(chunk))));
        }
      } else if !this.tail.is_empty() {
        let chunk: Vec<u8> = this.tail.drain(..).collect();
        return Poll::Ready(Some(Ok(Bytes::from(chunk))));
      }

      if *this.done && this.encoder.is_none() {
        return Poll::Ready(None);
      }

      match this.inner.as_mut().poll_next(cx) {
        Poll::Ready(Some(Ok(data))) => {
          if let Some(enc) = this.encoder.as_mut()
            && let Err(e) = enc.write_all(&data).and_then(|()| enc.flush())
          {
            return Poll::Ready(Some(Err(e.into())));
          }
        }
        Poll::Ready(Some(Err(e))) => {
          return Poll::Ready(Some(Err(e)));
        }
        Poll::Ready(None) => {
          *this.done = true;
          if let Some(enc) = this.encoder.take() {
            match enc.finish() {
              Ok(vec) => {
                *this.tail = vec;
                continue;
              }
              Err(e) => {
                return Poll::Ready(Some(Err(e.into())));
              }
            }
          }
          return Poll::Ready(None);
        }
        Poll::Pending => {
          return Poll::Pending;
        }
      }
    }
  }
}
