//! DEFLATE compression streaming utilities for efficient HTTP response compression.
//!
//! This module provides streaming DEFLATE compression for HTTP response bodies using the
//! flate2 crate. DEFLATE compression offers good compression ratios with fast processing
//! speeds, making it suitable for real-time web content compression. The streaming
//! implementation enables memory-efficient compression of large responses without
//! buffering entire content in memory.
//!
//! # Examples
//!
//! ```rust
//! use tako::plugins::compression::deflate_stream::stream_deflate;
//! use http_body_util::Full;
//! use bytes::Bytes;
//!
//! // Compress a response body with DEFLATE level 6
//! let body = Full::from(Bytes::from("Hello, World! This is test content."));
//! let compressed = stream_deflate(body, 6);
//!
//! // Fast compression for dynamic content
//! let dynamic_content = Full::from(Bytes::from("API response data..."));
//! let fast_compressed = stream_deflate(dynamic_content, 1);
//! ```

use std::io::Write;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use anyhow::Result;
use bytes::Bytes;
use flate2::Compression;
use flate2::write::DeflateEncoder;
use futures_util::Stream;
use futures_util::TryStreamExt;
use http_body::Body;
use http_body::Frame;
use http_body_util::BodyExt;
use pin_project_lite::pin_project;

use crate::body::TakoBody;
use crate::types::BoxError;

/// Compresses an HTTP body stream using the DEFLATE compression algorithm.
pub fn stream_deflate<B>(body: B, level: u32) -> TakoBody
where
  B: Body<Data = Bytes, Error = BoxError> + Send + 'static,
{
  let upstream = body.into_data_stream();
  let deflate = DeflateStream::new(upstream, level).map_ok(Frame::data);
  TakoBody::from_try_stream(deflate)
}

pin_project! {
    /// Streaming DEFLATE compressor that wraps an inner data stream.
    pub struct DeflateStream<S> {
        #[pin] inner: S,
        encoder: DeflateEncoder<Vec<u8>>,
        pos: usize,
        done: bool,
    }
}

impl<S> DeflateStream<S> {
  /// Creates a new DEFLATE compression stream with the specified compression level.
  pub fn new(inner: S, level: u32) -> Self {
    Self {
      inner,
      encoder: DeflateEncoder::new(Vec::new(), Compression::new(level)),
      pos: 0,
      done: false,
    }
  }
}

impl<S> Stream for DeflateStream<S>
where
  S: Stream<Item = Result<Bytes, BoxError>>,
{
  type Item = Result<Bytes, BoxError>;

  /// Polls the stream for the next compressed data chunk.
  fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    let mut this = self.project();

    loop {
      // If there is data in the encoder's buffer, return it.
      if *this.pos < this.encoder.get_ref().len() {
        let buf = &this.encoder.get_ref()[*this.pos..];
        *this.pos = this.encoder.get_ref().len();
        return Poll::Ready(Some(Ok(Bytes::copy_from_slice(buf))));
      }

      // If the stream is done, return None to indicate completion.
      if *this.done {
        return Poll::Ready(None);
      }

      // Poll the inner stream for the next chunk of data.
      match this.inner.as_mut().poll_next(cx) {
        Poll::Ready(Some(Ok(chunk))) => {
          // Compress the chunk and flush the encoder.
          if let Err(e) = this
            .encoder
            .write_all(&chunk)
            .and_then(|_| this.encoder.flush())
          {
            return Poll::Ready(Some(Err(e.into())));
          }
          continue;
        }
        Poll::Ready(Some(Err(e))) => {
          // Propagate errors from the inner stream.
          return Poll::Ready(Some(Err(e)));
        }
        Poll::Ready(None) => {
          // Finalize the compression when the inner stream is finished.
          *this.done = true;
          if let Err(e) = this.encoder.try_finish() {
            return Poll::Ready(Some(Err(e.into())));
          }
          continue;
        }
        Poll::Pending => {
          // Indicate that the stream is not ready yet.
          return Poll::Pending;
        }
      }
    }
  }
}
