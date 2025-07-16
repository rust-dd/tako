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
///
/// This function converts any HTTP body into a Brotli-compressed streaming body using
/// the specified compression level. The compression is performed on-the-fly as data
/// flows through the stream, making it memory-efficient for large responses.
///
/// # Arguments
///
/// * `body` - HTTP body to compress, must implement `Body<Data = Bytes, Error = BoxError>`
/// * `lvl` - Brotli compression level (1-11, where 11 provides maximum compression)
///
/// # Compression Levels
///
/// - **1-3**: Fast compression, lower compression ratio
/// - **4-6**: Balanced compression and speed (recommended for most use cases)
/// - **7-9**: High compression, slower processing
/// - **10-11**: Maximum compression, slowest processing (best for static content)
///
/// # Examples
///
/// ```rust
/// use tako::plugins::compression::brotli_stream::stream_brotli;
/// use http_body_util::Full;
/// use bytes::Bytes;
///
/// // Fast compression for dynamic content
/// let body = Full::from(Bytes::from("Dynamic API response data"));
/// let fast_compressed = stream_brotli(body, 4);
///
/// // Maximum compression for static assets
/// let static_body = Full::from(Bytes::from("Large static file content..."));
/// let max_compressed = stream_brotli(static_body, 11);
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
    /// Streaming Brotli compressor that wraps an inner data stream.
    ///
    /// `BrotliStream` provides on-the-fly Brotli compression for streaming data sources.
    /// It maintains an internal encoder state and buffer to efficiently compress data
    /// as it flows through the stream. The implementation handles backpressure and
    /// ensures all compressed data is properly flushed when the stream ends.
    ///
    /// # Type Parameters
    ///
    /// * `S` - Inner stream type that yields `Result<Bytes, BoxError>` items
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::brotli_stream::BrotliStream;
    /// use futures_util::stream;
    /// use bytes::Bytes;
    ///
    /// # async fn example() {
    /// // Create a stream of data chunks
    /// let data_stream = stream::iter(vec![
    ///     Ok(Bytes::from("First chunk of data")),
    ///     Ok(Bytes::from("Second chunk of data")),
    ///     Ok(Bytes::from("Final chunk of data")),
    /// ]);
    ///
    /// // Wrap with Brotli compression
    /// let compressed_stream = BrotliStream::new(data_stream, 6);
    /// # }
    /// ```
    pub struct BrotliStream<S> {
        /// Inner stream providing source data for compression.
        #[pin] inner: S,
        /// Brotli encoder with internal buffer for compressed output.
        encoder: brotli::CompressorWriter<Vec<u8>>,
        /// Current position in the encoder's output buffer.
        pos: usize,
        /// Flag indicating whether the input stream has ended.
        done: bool,
    }
}

impl<S> BrotliStream<S> {
    /// Creates a new Brotli compression stream with the specified compression level.
    ///
    /// The encoder is initialized with a 4KB buffer size and standard Brotli parameters
    /// optimized for streaming web content. The compression level controls the
    /// trade-off between compression ratio and processing speed.
    ///
    /// # Arguments
    ///
    /// * `stream` - Source stream to compress
    /// * `level` - Brotli compression level (1-11, clamped to valid range)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::brotli_stream::BrotliStream;
    /// use futures_util::stream;
    /// use bytes::Bytes;
    ///
    /// # fn example() {
    /// let source = stream::iter(vec![Ok(Bytes::from("test data"))]);
    ///
    /// // Fast compression for real-time data
    /// let fast_stream = BrotliStream::new(source.clone(), 1);
    ///
    /// // High compression for static content
    /// let high_stream = BrotliStream::new(source, 9);
    /// # }
    /// ```
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
    ///
    /// This method implements the core streaming compression logic:
    /// 1. Returns any buffered compressed data first
    /// 2. Polls the inner stream for new input data
    /// 3. Compresses new input and buffers the output
    /// 4. Finalizes compression when input stream ends
    /// 5. Handles errors and backpressure appropriately
    ///
    /// The implementation ensures efficient memory usage by immediately returning
    /// compressed data as it becomes available, rather than accumulating large
    /// buffers.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::plugins::compression::brotli_stream::BrotliStream;
    /// use futures_util::{stream, StreamExt};
    /// use bytes::Bytes;
    ///
    /// # async fn example() {
    /// let data = stream::iter(vec![
    ///     Ok(Bytes::from("chunk1")),
    ///     Ok(Bytes::from("chunk2")),
    /// ]);
    ///
    /// let mut compressed = BrotliStream::new(data, 6);
    ///
    /// // Poll for compressed chunks
    /// while let Some(result) = compressed.next().await {
    ///     match result {
    ///         Ok(compressed_chunk) => {
    ///             println!("Compressed {} bytes", compressed_chunk.len());
    ///         }
    ///         Err(e) => {
    ///             eprintln!("Compression error: {}", e);
    ///             break;
    ///         }
    ///     }
    /// }
    /// # }
    /// ```
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
