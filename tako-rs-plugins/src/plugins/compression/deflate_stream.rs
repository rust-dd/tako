//! # Examples
//!
//! ```rust
//! use tako::plugins::compression::deflate_stream::stream_deflate;
//! use tako::body::TakoBody;
//!
//! // Compress a response body with DEFLATE level 6
//! let body = TakoBody::from("Hello, World! This is test content.");
//! let compressed = stream_deflate(body, 6);
//!
//! // Fast compression for dynamic content
//! let dynamic_content = TakoBody::from("API response data...");
//! let fast_compressed = stream_deflate(dynamic_content, 1);
//! ```

use std::io::Write;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use anyhow::Result;
use bytes::Bytes;
use flate2::Compression;
use flate2::write::ZlibEncoder;
use futures_util::Stream;
use futures_util::TryStreamExt;
use http_body::Body;
use http_body::Frame;
use http_body_util::BodyExt;
use pin_project_lite::pin_project;
use tako_rs_core::body::TakoBody;
use tako_rs_core::types::BoxError;

/// Zlib-wrapped, as the HTTP `deflate` coding requires; flushes after every input chunk.
pub fn stream_deflate<B>(body: B, level: u32) -> TakoBody
where
  B: Body<Data = Bytes, Error = BoxError> + Send + 'static,
{
  let upstream = body.into_data_stream();
  let deflate = DeflateStream::new(upstream, level).map_ok(Frame::data);
  TakoBody::from_try_stream(deflate)
}

pin_project! {
    pub struct DeflateStream<S> {
        #[pin] inner: S,
        encoder: ZlibEncoder<Vec<u8>>,
        done: bool,
    }
}

impl<S> DeflateStream<S> {
  pub fn new(inner: S, level: u32) -> Self {
    Self {
      inner,
      encoder: ZlibEncoder::new(Vec::new(), Compression::new(level)),
      done: false,
    }
  }
}

impl<S> Stream for DeflateStream<S>
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
