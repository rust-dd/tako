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

use crate::{body::TakoBody, types::BoxError};

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
        encoder: brotli::CompressorWriter<Vec<u8>>,
        pos: usize,
        done: bool,
    }
}

impl<S> BrotliStream<S> {
    /// Creates a new Brotli compression stream with the specified compression level.
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
    type Item = Result<Bytes, BoxError>;

    /// Polls the stream for the next compressed data chunk.
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            // 1) Do we have unread bytes in the encoder's buffer?
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
                        return Poll::Ready(Some(Err(e.into())));
                    }
                    continue; // encoder now contains data → step 1
                }
                // Propagate an error from the inner stream.
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(e)));
                }
                // Inner stream ended: finalize the encoder, then loop to drain it.
                Poll::Ready(None) => {
                    *this.done = true;
                    if let Err(e) = this.encoder.flush() {
                        return Poll::Ready(Some(Err(e.into())));
                    }
                    continue; // encoder may hold final bytes → step 1
                }
                // Still waiting for more input and nothing buffered.
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        }
    }
}
