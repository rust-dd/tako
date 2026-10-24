//! # Examples
//!
//! ```rust
//! use tako::plugins::compression::gzip_stream::stream_gzip;
//! use tako::body::TakoBody;
//!
//! // Compress a response body with Gzip level 6
//! let body = TakoBody::from("Hello, World! This is test content.");
//! let compressed = stream_gzip(body, 6);
//!
//! // Fast compression for dynamic API responses
//! let api_response = TakoBody::from("JSON API data...");
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
use http_body::Body;
use pin_project_lite::pin_project;
use tako_rs_core::body::TakoBody;
use tako_rs_core::types::BoxError;

/// Flushes after every input chunk, trading compression ratio for latency.
pub fn stream_gzip<B>(body: B, level: u32) -> TakoBody
where
  B: Body<Data = Bytes, Error = BoxError> + Send + 'static,
{
  super::frames::encode(body, |stream| GzipStream::new(stream, level))
}

pin_project! {
    pub struct GzipStream<S> {
        #[pin] inner: S,
        encoder: GzEncoder<Vec<u8>>,
        done: bool,
    }
}

impl<S> GzipStream<S> {
  fn new(stream: S, level: u32) -> Self {
    Self {
      inner: stream,
      encoder: GzEncoder::new(Vec::new(), Compression::new(level)),
      done: false,
    }
  }
}

impl<S> Stream for GzipStream<S>
where
  S: Stream<Item = Result<Bytes, BoxError>>,
{
  type Item = Result<Bytes, BoxError>;

  fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    let mut this = self.project();

    loop {
      if !this.encoder.get_ref().is_empty() {
        let chunk = std::mem::take(this.encoder.get_mut());
        return Poll::Ready(Some(Ok(Bytes::from(chunk))));
      }
      if *this.done {
        return Poll::Ready(None);
      }
      match this.inner.as_mut().poll_next(cx) {
        Poll::Ready(Some(Ok(chunk))) => {
          if let Err(e) = this
            .encoder
            .write_all(&chunk)
            .and_then(|()| this.encoder.flush())
          {
            return Poll::Ready(Some(Err(e.into())));
          }
        }
        Poll::Ready(Some(Err(e))) => {
          return Poll::Ready(Some(Err(e)));
        }
        Poll::Ready(None) => {
          *this.done = true;
          // `try_finish`, not `flush`: only FINISH writes the CRC32/ISIZE trailer decoders need.
          if let Err(e) = this.encoder.try_finish() {
            return Poll::Ready(Some(Err(e.into())));
          }
        }
        Poll::Pending => {
          return Poll::Pending;
        }
      }
    }
  }
}
