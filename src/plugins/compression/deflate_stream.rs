//! DEFLATE compression streaming utilities for efficient HTTP response compression.
//!
//! This module provides streaming DEFLATE compression for HTTP response bodies using the
//! flate2 crate. DEFLATE compression offers good compression ratios with fast processing
//! speeds, making it suitable for real-time web content compression. The streaming
//! implementation enables memory-efficient compression of large responses without
//! buffering entire content in memory.
//!
//! # Examples
//!
//! ```rust
//! use tako::plugins::compression::deflate_stream::stream_deflate;
//! use http_body_util::Full;
//! use bytes::Bytes;
//!
//! // Compress a response body with DEFLATE level 6
//! let body = Full::from(Bytes::from("Hello, World! This is test content."));
//! let compressed = stream_deflate(body, 6);
//!
//! // Fast compression for dynamic content
//! let dynamic_content = Full::from(Bytes::from("API response data..."));
//! let fast_compressed = stream_deflate(dynamic_content, 1);
//! ```

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

/// Compresses an HTTP body stream using the DEFLATE compression algorithm.
///
/// This function converts any HTTP body into a DEFLATE-compressed streaming body using
/// the specified compression level. The compression is performed incrementally as data
/// flows through the stream, providing memory-efficient compression for responses of
/// any size.
///
/// # Arguments
///
/// * `body` - HTTP body to compress, must implement `Body<Data = Bytes, Error = BoxError>`
/// * `level` - DEFLATE compression level (0-9, where 9 provides maximum compression)
///
/// # Compression Levels
///
/// - **0**: No compression (store only)
/// - **1-3**: Fast compression, lower compression ratio
/// - **4-6**: Balanced compression and speed (recommended for most use cases)
/// - **7-9**: High compression, slower processing (best for static content)
///
/// # Examples
///
/// ```rust
/// use tako::plugins::compression::deflate_stream::stream_deflate;
/// use http_body_util::Full;
/// use bytes::Bytes;
///
/// // Balanced compression for general web content
/// let body = Full::from(Bytes::from("JSON API response data"));
/// let compressed = stream_deflate(body, 6);
///
/// // Fast compression for real-time data
/// let realtime_body = Full::from(Bytes::from("Live data stream"));
/// let fast_compressed = stream_deflate(realtime_body, 1);
///
/// // Maximum compression for static assets
/// let static_body = Full::from(Bytes::from("Large static file content..."));
/// let max_compressed = stream_deflate(static_body, 9);
/// ```
pub fn stream_deflate<B>(body: B, level: u32) -> TakoBody
where
    B: Body<Data = Bytes, Error = BoxError> + Send + 'static,
{
    let upstream = body.into_data_stream();
    let deflate = DeflateStream::new(upstream, level).map_ok(Frame::data);
    TakoBody::from_try_stream(deflate)
}

pin_project! {
    /// Streaming DEFLATE compressor that wraps an inner data stream.
    ///
    /// `DeflateStream` provides on-the-fly DEFLATE compression for streaming data sources.
    /// It maintains an internal encoder state and buffer to efficiently compress data
    /// as it flows through the stream. The implementation handles proper stream
    /// finalization and ensures all compressed data is flushed when the input ends.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::deflate_stream::DeflateStream;
    /// use futures_util::stream;
    /// use bytes::Bytes;
    ///
    /// # fn example() {
    /// // Create a stream of data chunks
    /// let data_stream = stream::iter(vec![
    ///     Ok(Bytes::from("First data chunk")),
    ///     Ok(Bytes::from("Second data chunk")),
    ///     Ok(Bytes::from("Final data chunk")),
    /// ]);
    ///
    /// // Wrap with DEFLATE compression at level 6
    /// let compressed_stream = DeflateStream::new(data_stream, 6);
    /// # }
    /// ```
    pub struct DeflateStream<S> {
        /// Inner stream providing source data for compression.
        #[pin] inner: S,
        /// DEFLATE encoder with internal buffer for compressed output.
        encoder: DeflateEncoder<Vec<u8>>,
        /// Current position in the encoder's output buffer.
        pos: usize,
        /// Flag indicating whether the input stream has ended.
        done: bool,
    }
}

impl<S> DeflateStream<S> {
    /// Creates a new DEFLATE compression stream with the specified compression level.
    ///
    /// The encoder is initialized with the specified compression level, which controls
    /// the trade-off between compression ratio and processing speed. Higher levels
    /// provide better compression at the cost of increased CPU usage.
    ///
    /// # Arguments
    ///
    /// * `inner` - Source stream to compress
    /// * `level` - DEFLATE compression level (0-9, clamped to valid range)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::deflate_stream::DeflateStream;
    /// use futures_util::stream;
    /// use bytes::Bytes;
    ///
    /// # fn example() {
    /// let source = stream::iter(vec![Ok(Bytes::from("test data"))]);
    ///
    /// // Fast compression for dynamic content
    /// let fast_stream = DeflateStream::new(source.clone(), 1);
    ///
    /// // Balanced compression for general use
    /// let balanced_stream = DeflateStream::new(source.clone(), 6);
    ///
    /// // Maximum compression for static content
    /// let max_stream = DeflateStream::new(source, 9);
    /// # }
    /// ```
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

    /// Polls the stream for the next compressed data chunk.
    ///
    /// This method implements the core streaming compression logic with the following steps:
    /// 1. Returns any buffered compressed data immediately if available
    /// 2. Polls the inner stream for new input data when buffer is empty
    /// 3. Compresses new input data and flushes it to the buffer
    /// 4. Finalizes compression when the input stream ends
    /// 5. Handles errors and backpressure appropriately
    ///
    /// The implementation prioritizes memory efficiency by returning compressed data
    /// as soon as it's available rather than accumulating large buffers.
    ///
    /// # Returns
    ///
    /// - `Poll::Ready(Some(Ok(Bytes)))` - A compressed chunk of data
    /// - `Poll::Ready(Some(Err(BoxError)))` - An error occurred during compression
    /// - `Poll::Ready(None)` - The stream has finished and all data has been compressed
    /// - `Poll::Pending` - The stream is not ready and should be polled again later
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::plugins::compression::deflate_stream::DeflateStream;
    /// use futures_util::{stream, StreamExt};
    /// use bytes::Bytes;
    ///
    /// # async fn example() {
    /// let data = stream::iter(vec![
    ///     Ok(Bytes::from("chunk1")),
    ///     Ok(Bytes::from("chunk2")),
    /// ]);
    ///
    /// let mut compressed = DeflateStream::new(data, 6);
    ///
    /// // Process compressed chunks as they become available
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
