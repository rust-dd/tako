//! Gzip compression streaming utilities for efficient HTTP response compression.
//!
//! This module provides streaming Gzip compression for HTTP response bodies using the
//! flate2 crate. Gzip is one of the most widely supported compression formats on the web,
//! offering excellent compatibility across all browsers and HTTP clients. The streaming
//! implementation enables memory-efficient compression of large responses without
//! buffering entire content in memory, making it ideal for real-time web applications.
//!
//! # Examples
//!
//! ```rust
//! use tako::plugins::compression::gzip_stream::stream_gzip;
//! use http_body_util::Full;
//! use bytes::Bytes;
//!
//! // Compress a response body with Gzip level 6
//! let body = Full::from(Bytes::from("Hello, World! This is test content."));
//! let compressed = stream_gzip(body, 6);
//!
//! // Fast compression for dynamic API responses
//! let api_response = Full::from(Bytes::from("JSON API data..."));
//! let fast_compressed = stream_gzip(api_response, 1);
//! ```

use std::io::Write;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use anyhow::Result;
use bytes::Bytes;
use flate2::Compression;
use flate2::write::GzEncoder;
use futures_util::Stream;
use futures_util::TryStreamExt;
use http_body::Body;
use http_body::Frame;
use http_body_util::BodyExt;
use pin_project_lite::pin_project;

use crate::body::TakoBody;
use crate::types::BoxError;

/// Compresses an HTTP body stream using Gzip compression algorithm.
pub fn stream_gzip<B>(body: B, level: u32) -> TakoBody
where
  B: Body<Data = Bytes, Error = BoxError> + Send + 'static,
{
  let upstream = body.into_data_stream();
  let gzip = GzipStream::new(upstream, level).map_ok(Frame::data);
  TakoBody::from_try_stream(gzip)
}

pin_project! {
    /// Streaming Gzip compressor that wraps an inner data stream.
    pub struct GzipStream<S> {
        #[pin] inner: S,
        encoder: GzEncoder<Vec<u8>>,
        pos: usize,
        done: bool,
    }
}

impl<S> GzipStream<S> {
  /// Creates a new Gzip compression stream with the specified compression level.
  fn new(stream: S, level: u32) -> Self {
    Self {
      inner: stream,
      encoder: GzEncoder::new(Vec::new(), Compression::new(level)),
      pos: 0,
      done: false,
    }
  }
}

impl<S> Stream for GzipStream<S>
where
  S: Stream<Item = Result<Bytes, BoxError>>,
{
  type Item = Result<Bytes, BoxError>;

  /// Polls the stream for the next compressed data chunk.
  fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    let mut this = self.project();

    loop {
      // 1) Do we still have unread bytes in the encoder buffer?
      if *this.pos < this.encoder.get_ref().len() {
        let buf = &this.encoder.get_ref()[*this.pos..];
        *this.pos = this.encoder.get_ref().len();
        // Immediately send the chunk and return Ready.
        return Poll::Ready(Some(Ok(Bytes::copy_from_slice(buf))));
      }
      // 2) If we already finished and nothing is left, end the stream.
      if *this.done {
        return Poll::Ready(None);
      }
      // 3) Poll the inner stream for more input data.
      match this.inner.as_mut().poll_next(cx) {
        // New chunk arrived: compress it, then loop again
        // (now the buffer certainly contains data).
        Poll::Ready(Some(Ok(chunk))) => {
          if let Err(e) = this
            .encoder
            .write_all(&chunk)
            .and_then(|_| this.encoder.flush())
          {
            return Poll::Ready(Some(Err(e.into())));
          }
          continue;
        }
        // Error from the inner stream — propagate it.
        Poll::Ready(Some(Err(e))) => {
          return Poll::Ready(Some(Err(e)));
        }
        // Inner stream finished: finalize the encoder,
        // then loop to drain the remaining bytes.
        Poll::Ready(None) => {
          *this.done = true;
          if let Err(e) = this.encoder.flush() {
            return Poll::Ready(Some(Err(e.into())));
          }
          continue;
        }
        // No new input and no buffered output: we must wait.
        Poll::Pending => {
          return Poll::Pending;
        }
      }
    }
  }
}
