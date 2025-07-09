use std::{
    io::Write,
    pin::Pin,
    task::{Context, Poll},
};

use anyhow::Result;
use bytes::Bytes;
use flate2::{Compression, write::DeflateEncoder};
use futures_util::{Stream, TryStreamExt};
use http_body_util::BodyExt;
use hyper::body::{Body, Frame};
use pin_project_lite::pin_project;

use crate::{body::TakoBody, types::BoxError};

/// Compresses a given body stream using the DEFLATE algorithm and returns a `TakoBody`.
///
/// # Arguments
/// - `body`: The input body stream to be compressed.
/// - `level`: The compression level (0-9, where 0 is no compression and 9 is maximum compression).
///
/// # Returns
/// A `TakoBody` containing the compressed stream.
pub fn stream_deflate<B>(body: B, level: u32) -> TakoBody
where
    B: Body<Data = Bytes, Error = BoxError> + Send + 'static,
{
    let upstream = body.into_data_stream();
    let deflate = DeflateStream::new(upstream, level).map_ok(Frame::data);
    TakoBody::from_try_stream(deflate)
}

pin_project! {
    /// A stream that compresses data using the DEFLATE algorithm.
    ///
    /// This struct wraps an inner stream and applies DEFLATE compression to its output.
    pub struct DeflateStream<S> {
        #[pin] inner: S,
        encoder: DeflateEncoder<Vec<u8>>,
        pos: usize,
        done: bool,
    }
}

impl<S> DeflateStream<S> {
    /// Creates a new `DeflateStream` with the specified inner stream and compression level.
    ///
    /// # Arguments
    /// - `inner`: The inner stream to be compressed.
    /// - `level`: The compression level (0-9, where 0 is no compression and 9 is maximum compression).
    ///
    /// # Returns
    /// A new `DeflateStream` instance.
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

    /// Polls the next compressed chunk from the stream.
    ///
    /// This method handles the following:
    /// - If there is data in the encoder's buffer, it returns the next chunk.
    /// - If the inner stream has more data, it compresses it and continues.
    /// - If the inner stream is finished, it finalizes the compression and returns the remaining data.
    ///
    /// # Arguments
    /// - `cx`: The context for the asynchronous operation.
    ///
    /// # Returns
    /// - `Poll::Ready(Some(Ok(Bytes)))`: A compressed chunk of data.
    /// - `Poll::Ready(Some(Err(BoxError)))`: An error occurred during compression.
    /// - `Poll::Ready(None)`: The stream has finished.
    /// - `Poll::Pending`: The stream is not ready yet.
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
