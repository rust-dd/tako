use std::{
    io::Write,
    pin::Pin,
    task::{Context, Poll},
};

use anyhow::Result;
use bytes::Bytes;
use futures_util::{Stream, TryStreamExt};
use http_body_util::BodyExt;
use hyper::body::{Body, Frame};
use pin_project_lite::pin_project;
use tokio::io;

use crate::{body::TakoBody, types::BoxError};

/// Compresses an HTTP body stream using Brotli compression.
///
/// # Arguments
///
/// * `body` - The HTTP body to compress, which must implement the `Body` trait.
/// * `lvl` - The Brotli compression level (1-11, where 11 is the highest compression).
///
/// # Returns
///
/// A `TakoBody` containing the compressed stream.
///
/// # Example
///
/// ```rust
/// use tako::plugins::compression::stream_brotli;
/// use hyper::Body;
///
/// let body = Body::from("example data");
/// let compressed_body = stream_brotli(body, 5);
/// ```
pub fn stream_brotli<B>(body: B, lvl: u32) -> TakoBody
where
    B: Body<Data = Bytes, Error = BoxError> + Send + 'static,
{
    let stream = body.into_data_stream();
    let stream = BrotliStream::new(stream, lvl).map_ok(Frame::data);
    TakoBody::from_try_stream(stream)
}

pin_project! {
    /// A stream that compresses data using Brotli compression.
    ///
    /// This struct wraps an inner stream and compresses its output on the fly
    /// using the Brotli algorithm. It is designed to be used with asynchronous
    /// streams of data.
    ///
    /// # Type Parameters
    ///
    /// * `S` - The type of the inner stream, which must implement `Stream` and
    ///   yield `Result<Bytes, BoxError>`.
    pub struct BrotliStream<S> {
        #[pin] inner: S,
        encoder: brotli::CompressorWriter<Vec<u8>>,
        pos: usize,
        done: bool,
    }
}

impl<S> BrotliStream<S> {
    /// Creates a new `BrotliStream` with the specified inner stream and compression level.
    ///
    /// # Arguments
    ///
    /// * `stream` - The inner stream to wrap and compress.
    /// * `level` - The Brotli compression level (1-11, where 11 is the highest compression).
    ///
    /// # Returns
    ///
    /// A new `BrotliStream` instance.
    fn new(stream: S, level: u32) -> Self {
        Self {
            inner: stream,
            encoder: brotli::CompressorWriter::new(Vec::new(), 4096, level, 22),
            pos: 0,
            done: false,
        }
    }
}

impl<S> Stream for BrotliStream<S>
where
    S: Stream<Item = Result<Bytes, BoxError>>,
{
    type Item = Result<Bytes, io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            // 1) Do we have unread bytes in the encoder’s buffer?
            if *this.pos < this.encoder.get_ref().len() {
                let buf = &this.encoder.get_ref()[*this.pos..];
                *this.pos = this.encoder.get_ref().len();
                // Immediately return the data.
                return Poll::Ready(Some(Ok(Bytes::copy_from_slice(buf))));
            }
            // 2) Encoder is drained and we already finalized ⇒ stream is over.
            if *this.done {
                return Poll::Ready(None);
            }
            // 3) Poll the inner stream for more input.
            match this.inner.as_mut().poll_next(cx) {
                // Got a new chunk: compress it and loop to flush it out.
                Poll::Ready(Some(Ok(chunk))) => {
                    if let Err(e) = this
                        .encoder
                        .write_all(&chunk)
                        .and_then(|_| this.encoder.flush())
                    {
                        return Poll::Ready(Some(Err(e)));
                    }
                    continue; // encoder now contains data → step 1
                }
                // Propagate an error from the inner stream.
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(io::Error::new(io::ErrorKind::Other, e))));
                }
                // Inner stream ended: finalize the encoder, then loop to drain it.
                Poll::Ready(None) => {
                    *this.done = true;
                    if let Err(e) = this.encoder.flush() {
                        return Poll::Ready(Some(Err(e)));
                    }
                    continue; // encoder may hold final bytes → step 1
                }
                // Still waiting for more input and nothing buffered.
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}
