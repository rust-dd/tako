use std::{
    io::Write,
    pin::Pin,
    task::{Context, Poll},
};

use anyhow::Result;
use bytes::Bytes;
use flate2::{Compression, write::GzEncoder};
use futures_util::{Stream, TryStreamExt};
use http_body_util::BodyExt;
use hyper::body::{Body, Frame};
use pin_project_lite::pin_project;

use crate::{body::TakoBody, types::BoxError};

/// Compresses an HTTP body stream using Gzip compression.
///
/// # Arguments
///
/// * `body` - The HTTP body to compress, which must implement the `Body` trait.
/// * `level` - The compression level to use (1-9, where 9 is the highest compression).
///
/// # Returns
///
/// A `TakoBody` containing the Gzip-compressed stream.
///
/// # Example
///
/// ```rust
/// use tako::plugins::compression::stream_gzip;
/// use hyper::Body;
/// use bytes::Bytes;
///
/// let body = Body::from(Bytes::from("Hello, world!"));
/// let compressed_body = stream_gzip(body, 6);
/// ```
pub fn stream_gzip<B>(body: B, level: u32) -> TakoBody
where
    B: Body<Data = Bytes, Error = BoxError> + Send + 'static,
{
    let upstream = body.into_data_stream();
    let gzip = GzipStream::new(upstream, level).map_ok(Frame::data);
    TakoBody::from_try_stream(gzip)
}

pin_project! {
    /// A stream that compresses data using Gzip compression.
    ///
    /// This struct wraps an inner stream and compresses its output on-the-fly
    /// using the Gzip algorithm. It is designed to be used with asynchronous
    /// streams in Rust.
    ///
    /// # Type Parameters
    ///
    /// * `S` - The inner stream that provides the data to be compressed.
    ///
    /// # Example
    ///
    /// ```rust
    /// use tako::plugins::compression::GzipStream;
    /// use futures_util::stream;
    /// use bytes::Bytes;
    ///
    /// let data_stream = stream::iter(vec![Ok(Bytes::from("Hello")), Ok(Bytes::from("World"))]);
    /// let gzip_stream = GzipStream::new(data_stream, 6);
    /// ```
    pub struct GzipStream<S> {
        #[pin] inner: S,
        encoder: GzEncoder<Vec<u8>>,
        pos: usize,
        done: bool,
    }
}

impl<S> GzipStream<S> {
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
                        return Poll::Ready(Some(Err(Box::new(e))));
                    }
                    continue;
                }
                // Error from the inner stream — propagate it.
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e))),
                // Inner stream finished: finalize the encoder,
                // then loop to drain the remaining bytes.
                Poll::Ready(None) => {
                    *this.done = true;
                    if let Err(e) = this.encoder.try_finish() {
                        return Poll::Ready(Some(Err(Box::new(e))));
                    }
                    continue;
                }
                // No new input and no buffered output: we must wait.
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}
