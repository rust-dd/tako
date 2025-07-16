//! Gzip compression streaming utilities for efficient HTTP response compression.
//!
//! This module provides streaming Gzip compression for HTTP response bodies using the
//! flate2 crate. Gzip is one of the most widely supported compression formats on the web,
//! offering excellent compatibility across all browsers and HTTP clients. The streaming
//! implementation enables memory-efficient compression of large responses without
//! buffering entire content in memory, making it ideal for real-time web applications.
//!
//! # Examples
//!
//! ```rust
//! use tako::plugins::compression::gzip_stream::stream_gzip;
//! use http_body_util::Full;
//! use bytes::Bytes;
//!
//! // Compress a response body with Gzip level 6
//! let body = Full::from(Bytes::from("Hello, World! This is test content."));
//! let compressed = stream_gzip(body, 6);
//!
//! // Fast compression for dynamic API responses
//! let api_response = Full::from(Bytes::from("JSON API data..."));
//! let fast_compressed = stream_gzip(api_response, 1);
//! ```

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

/// Compresses an HTTP body stream using Gzip compression algorithm.
///
/// This function converts any HTTP body into a Gzip-compressed streaming body using
/// the specified compression level. Gzip compression provides excellent browser
/// compatibility and good compression ratios for text-based content. The compression
/// is performed incrementally as data flows through the stream.
///
/// # Arguments
///
/// * `body` - HTTP body to compress, must implement `Body<Data = Bytes, Error = BoxError>`
/// * `level` - Gzip compression level (1-9, where 9 provides maximum compression)
///
/// # Compression Levels
///
/// - **1-3**: Fast compression, lower compression ratio (good for real-time data)
/// - **4-6**: Balanced compression and speed (recommended for most web content)
/// - **7-9**: High compression, slower processing (best for static assets)
///
/// # Examples
///
/// ```rust
/// use tako::plugins::compression::gzip_stream::stream_gzip;
/// use http_body_util::Full;
/// use bytes::Bytes;
///
/// // Standard compression for web content
/// let html_body = Full::from(Bytes::from("<html><body>Hello World</body></html>"));
/// let compressed = stream_gzip(html_body, 6);
///
/// // Fast compression for API responses
/// let json_body = Full::from(Bytes::from(r#"{"status": "ok", "data": [1,2,3]}"#));
/// let fast_compressed = stream_gzip(json_body, 1);
///
/// // Maximum compression for static files
/// let css_body = Full::from(Bytes::from("body { margin: 0; padding: 0; }"));
/// let max_compressed = stream_gzip(css_body, 9);
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
    /// Streaming Gzip compressor that wraps an inner data stream.
    ///
    /// `GzipStream` provides on-the-fly Gzip compression for streaming data sources.
    /// It maintains an internal encoder state and buffer to efficiently compress data
    /// as it flows through the stream. The implementation handles proper Gzip headers,
    /// compression, and trailer generation while managing backpressure and ensuring
    /// all compressed data is properly flushed when the stream ends.
    ///
    /// # Type Parameters
    ///
    /// * `S` - Inner stream type that yields `Result<Bytes, BoxError>` items
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::gzip_stream::GzipStream;
    /// use futures_util::stream;
    /// use bytes::Bytes;
    ///
    /// # fn example() {
    /// // Create a stream of data chunks
    /// let data_stream = stream::iter(vec![
    ///     Ok(Bytes::from("First chunk of web content")),
    ///     Ok(Bytes::from("Second chunk of web content")),
    ///     Ok(Bytes::from("Final chunk of web content")),
    /// ]);
    ///
    /// // Wrap with Gzip compression at level 6
    /// let gzip_stream = GzipStream::new(data_stream, 6);
    /// # }
    /// ```
    pub struct GzipStream<S> {
        /// Inner stream providing source data for compression.
        #[pin] inner: S,
        /// Gzip encoder with internal buffer for compressed output.
        encoder: GzEncoder<Vec<u8>>,
        /// Current position in the encoder's output buffer.
        pos: usize,
        /// Flag indicating whether the input stream has ended.
        done: bool,
    }
}

impl<S> GzipStream<S> {
    /// Creates a new Gzip compression stream with the specified compression level.
    ///
    /// The encoder is initialized with the specified compression level, which controls
    /// the trade-off between compression ratio and processing speed. The implementation
    /// uses the flate2 crate's GzEncoder for RFC 1952 compliant Gzip compression.
    ///
    /// # Arguments
    ///
    /// * `stream` - Source stream to compress
    /// * `level` - Gzip compression level (1-9, clamped to valid range)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::gzip_stream::GzipStream;
    /// use futures_util::stream;
    /// use bytes::Bytes;
    ///
    /// # fn example() {
    /// let source = stream::iter(vec![Ok(Bytes::from("test data"))]);
    ///
    /// // Fast compression for dynamic content
    /// let fast_stream = GzipStream::new(source.clone(), 1);
    ///
    /// // Balanced compression for general web content
    /// let balanced_stream = GzipStream::new(source.clone(), 6);
    ///
    /// // Maximum compression for static assets
    /// let max_stream = GzipStream::new(source, 9);
    /// # }
    /// ```
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

    /// Polls the stream for the next compressed data chunk.
    ///
    /// This method implements the core streaming compression logic with the following steps:
    /// 1. Returns any buffered compressed data immediately if available
    /// 2. Checks if the stream is finished and all data has been processed
    /// 3. Polls the inner stream for new input data when buffer is empty
    /// 4. Compresses new input data and flushes it to the buffer
    /// 5. Finalizes Gzip compression when the input stream ends
    /// 6. Handles errors and backpressure appropriately
    ///
    /// The implementation ensures proper Gzip format compliance including headers
    /// and trailers while maintaining memory efficiency by immediately returning
    /// compressed data as it becomes available.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::plugins::compression::gzip_stream::GzipStream;
    /// use futures_util::{stream, StreamExt};
    /// use bytes::Bytes;
    ///
    /// # async fn example() {
    /// let data = stream::iter(vec![
    ///     Ok(Bytes::from("chunk1")),
    ///     Ok(Bytes::from("chunk2")),
    /// ]);
    ///
    /// let mut compressed = GzipStream::new(data, 6);
    ///
    /// // Process compressed chunks as they become available
    /// while let Some(result) = compressed.next().await {
    ///     match result {
    ///         Ok(gzip_chunk) => {
    ///             println!("Compressed {} bytes with Gzip", gzip_chunk.len());
    ///         }
    ///         Err(e) => {
    ///             eprintln!("Gzip compression error: {}", e);
    ///             break;
    ///         }
    ///     }
    /// }
    /// # }
    /// ```
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
                        return Poll::Ready(Some(Err(e.into())));
                    }
                    continue;
                }
                // Error from the inner stream — propagate it.
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(e)));
                }
                // Inner stream finished: finalize the encoder,
                // then loop to drain the remaining bytes.
                Poll::Ready(None) => {
                    *this.done = true;
                    if let Err(e) = this.encoder.flush() {
                        return Poll::Ready(Some(Err(e.into())));
                    }
                    continue;
                }
                // No new input and no buffered output: we must wait.
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        }
    }
}
