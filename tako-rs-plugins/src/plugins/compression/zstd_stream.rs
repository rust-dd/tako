#![cfg(feature = "zstd")]

//! # Examples
//!
//! ```rust
//! # #[cfg(feature = "zstd")]
//! use tako::plugins::compression::zstd_stream::stream_zstd;
//! # #[cfg(feature = "zstd")]
//! use tako::body::TakoBody;
//! # #[cfg(feature = "zstd")]
//!
//! # #[cfg(feature = "zstd")]
//! # fn example() {
//! // Compress a response body with Zstandard level 3
//! let body = TakoBody::from("Hello, World! This is test content.");
//! let compressed = stream_zstd(body, 3);
//!
//! // High compression for static assets
//! let static_content = TakoBody::from("Large static file content...");
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
use tako_rs_core::body::TakoBody;
use tako_rs_core::types::BoxError;
use zstd::stream::Encoder;

/// Flushes after every input chunk, trading compression ratio for latency.
pub fn stream_zstd<B>(body: B, level: i32) -> TakoBody
where
  B: Body<Data = Bytes, Error = BoxError> + Send + 'static,
{
  let upstream = body.into_data_stream();
  let zstd_stream = ZstdStream::new(upstream, level).map_ok(Frame::data);
  TakoBody::from_try_stream(zstd_stream)
}

pin_project! {
    pub struct ZstdStream<S> {
        #[pin] inner: S,
        // Taken at EOF, since `finish` consumes it.
        encoder: Option<Encoder<'static, Vec<u8>>>,
        tail: Vec<u8>,
    }
}

impl<S> ZstdStream<S> {
  fn new(stream: S, level: i32) -> Self {
    Self {
      inner: stream,
      encoder: Some(Encoder::new(Vec::new(), level).expect("zstd encoder")),
      tail: Vec::new(),
    }
  }
}

impl<S> Stream for ZstdStream<S>
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
        Poll::Ready(Some(Ok(data))) => {
          if let Err(e) = enc.write_all(&data).and_then(|()| enc.flush()) {
            return Poll::Ready(Some(Err(e.into())));
          }
        }
        Poll::Ready(Some(Err(e))) => {
          return Poll::Ready(Some(Err(e)));
        }
        Poll::Ready(None) => {
          if let Some(enc) = this.encoder.take() {
            match enc.finish() {
              Ok(vec) => *this.tail = vec,
              Err(e) => return Poll::Ready(Some(Err(e.into()))),
            }
          }
        }
        Poll::Pending => {
          return Poll::Pending;
        }
      }
    }
  }
}
