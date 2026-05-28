//! Brotli compression streaming utilities for efficient HTTP response compression.
//!
//! This module provides streaming Brotli compression for HTTP response bodies, enabling
//! efficient compression of large responses without loading entire content into memory.
//! The implementation uses the brotli crate for high-quality compression with configurable
//! compression levels. Streaming compression is ideal for large responses, real-time data,
//! or memory-constrained environments.
//!
//! # Examples
//!
//! ```rust
//! use tako::plugins::compression::brotli_stream::stream_brotli;
//! use tako::body::TakoBody;
//! use http_body_util::Full;
//! use bytes::Bytes;
//!
//! // Compress a simple body with Brotli level 6
//! let body = Full::from(Bytes::from("Hello, World! This is some test data."));
//! let compressed = stream_brotli(body, 6);
//!
//! // High compression for static content
//! let static_content = Full::from(Bytes::from("Large static content here..."));
//! let high_compression = stream_brotli(static_content, 11);
//! ```

use std::io::Write;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use anyhow::Result;
use bytes::Bytes;
use futures_util::Stream;
use futures_util::TryStreamExt;
use http_body::Body;
use http_body::Frame;
use http_body_util::BodyExt;
use pin_project_lite::pin_project;
use tako_core::body::TakoBody;
use tako_core::types::BoxError;

/// Compresses an HTTP body stream using Brotli compression algorithm.
pub fn stream_brotli<B>(body: B, lvl: u32) -> TakoBody
where
  B: Body<Data = Bytes, Error = BoxError> + Send + 'static,
{
  let stream = body.into_data_stream();
  let stream = BrotliStream::new(stream, lvl).map_ok(Frame::data);
  TakoBody::from_try_stream(stream)
}

pin_project! {
    /// Streaming Brotli compressor that wraps an inner data stream.
    pub struct BrotliStream<S> {
        #[pin] inner: S,
        // `Option` so we can `take()` the encoder at EOF and call
        // `into_inner()` — that path emits the terminal IsLast metablock
        // (`BROTLI_OPERATION_FINISH`). Plain `flush()` only sends
        // `BROTLI_OPERATION_FLUSH`, which never closes the stream.
        encoder: Option<brotli::CompressorWriter<Vec<u8>>>,
        // Bytes produced by `encoder.into_inner()` at EOF — held separately
        // because the encoder is consumed at that point.
        tail: Vec<u8>,
        done: bool,
    }
}

impl<S> BrotliStream<S> {
  /// Creates a new Brotli compression stream with the specified compression level.
  fn new(stream: S, level: u32) -> Self {
    Self {
      inner: stream,
      encoder: Some(brotli::CompressorWriter::new(Vec::new(), 4096, level, 22)),
      tail: Vec::new(),
      done: false,
    }
  }
}

impl<S> Stream for BrotliStream<S>
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

      // 3) Poll the inner stream for more input.
      match this.inner.as_mut().poll_next(cx) {
        Poll::Ready(Some(Ok(chunk))) => {
          if let Some(enc) = this.encoder.as_mut()
            && let Err(e) = enc.write_all(&chunk).and_then(|()| enc.flush())
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
            // `into_inner` runs `BROTLI_OPERATION_FINISH` and returns the
            // inner `Vec<u8>` containing every byte the encoder ever wrote,
            // including the IsLast metablock. We've already drained the
            // encoder's live buffer above, so the returned vec only carries
            // anything emitted by the FINISH call itself.
            *this.tail = enc.into_inner();
            continue;
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
