//! # Examples
//!
//! ```rust
//! use tako::plugins::compression::brotli_stream::stream_brotli;
//! use tako::body::TakoBody;
//!
//! // Compress a simple body with Brotli level 6
//! let body = TakoBody::from("Hello, World! This is some test data.");
//! let compressed = stream_brotli(body, 6);
//!
//! // High compression for static content
//! let static_content = TakoBody::from("Large static content here...");
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
use tako_rs_core::body::TakoBody;
use tako_rs_core::types::BoxError;

/// Flushes after every input chunk, trading compression ratio for latency.
pub fn stream_brotli<B>(body: B, lvl: u32) -> TakoBody
where
  B: Body<Data = Bytes, Error = BoxError> + Send + 'static,
{
  let stream = body.into_data_stream();
  let stream = BrotliStream::new(stream, lvl).map_ok(Frame::data);
  TakoBody::from_try_stream(stream)
}

pin_project! {
    pub struct BrotliStream<S> {
        #[pin] inner: S,
        // Taken at EOF: only `into_inner` runs FINISH and writes the closing IsLast metablock.
        encoder: Option<brotli::CompressorWriter<Vec<u8>>>,
        tail: Vec<u8>,
    }
}

impl<S> BrotliStream<S> {
  fn new(stream: S, level: u32) -> Self {
    Self {
      inner: stream,
      encoder: Some(brotli::CompressorWriter::new(Vec::new(), 4096, level, 22)),
      tail: Vec::new(),
    }
  }
}

impl<S> Stream for BrotliStream<S>
where
  S: Stream<Item = Result<Bytes, BoxError>>,
{
  type Item = Result<Bytes, BoxError>;

  fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    let mut this = self.project();

    loop {
      let Some(enc) = this.encoder.as_mut() else {
        if this.tail.is_empty() {
          return Poll::Ready(None);
        }
        return Poll::Ready(Some(Ok(Bytes::from(std::mem::take(this.tail)))));
      };
      if !enc.get_ref().is_empty() {
        let chunk = std::mem::take(enc.get_mut());
        return Poll::Ready(Some(Ok(Bytes::from(chunk))));
      }

      match this.inner.as_mut().poll_next(cx) {
        Poll::Ready(Some(Ok(chunk))) => {
          if let Err(e) = enc.write_all(&chunk).and_then(|()| enc.flush()) {
            return Poll::Ready(Some(Err(e.into())));
          }
        }
        Poll::Ready(Some(Err(e))) => {
          return Poll::Ready(Some(Err(e)));
        }
        Poll::Ready(None) => {
          // The live buffer is drained, so this holds only what FINISH wrote.
          if let Some(enc) = this.encoder.take() {
            *this.tail = enc.into_inner();
          }
        }
        Poll::Pending => {
          return Poll::Pending;
        }
      }
    }
  }
}
