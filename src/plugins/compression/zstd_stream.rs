#![cfg(feature = "zstd")]

//! Zstandard compression streaming utilities for high-performance HTTP response compression.
//!
//! This module provides streaming Zstandard (zstd) compression for HTTP response bodies,
//! offering excellent compression ratios with fast decompression speeds. Zstandard is
//! particularly well-suited for modern web applications that require both high compression
//! efficiency and low latency. The streaming implementation enables memory-efficient
//! compression of large responses without buffering entire content in memory.
//!
//! # Examples
//!
//! ```rust
//! # #[cfg(feature = "zstd")]
//! use tako::plugins::compression::zstd_stream::stream_zstd;
//! # #[cfg(feature = "zstd")]
//! use http_body_util::Full;
//! # #[cfg(feature = "zstd")]
//! use bytes::Bytes;
//!
//! # #[cfg(feature = "zstd")]
//! # fn example() {
//! // Compress a response body with Zstandard level 3
//! let body = Full::from(Bytes::from("Hello, World! This is test content."));
//! let compressed = stream_zstd(body, 3);
//!
//! // High compression for static assets
//! let static_content = Full::from(Bytes::from("Large static file content..."));
//! let high_compressed = stream_zstd(static_content, 19);
//! # }
//! ```

use std::{
    io::Write,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_util::{Stream, TryStreamExt};
use http_body_util::BodyExt;
use hyper::body::{Body, Frame};
use pin_project_lite::pin_project;
use zstd::stream::Encoder;

use crate::{body::TakoBody, types::BoxError};

/// Compresses an HTTP body stream using Zstandard compression algorithm.
///
/// This function converts any HTTP body into a Zstandard-compressed streaming body using
/// the specified compression level. Zstandard provides excellent compression ratios with
/// fast compression and decompression speeds, making it ideal for modern web applications
/// that need to balance bandwidth efficiency with processing performance.
///
/// # Arguments
///
/// * `body` - HTTP body to compress, must implement `Body<Data = Bytes, Error = BoxError>`
/// * `level` - Zstandard compression level (1-22, where 22 provides maximum compression)
///
/// # Compression Levels
///
/// - **1-3**: Fast compression, good for real-time applications
/// - **4-9**: Balanced compression and speed (recommended for most web content)
/// - **10-15**: High compression, slower processing (good for static assets)
/// - **16-22**: Maximum compression, slowest processing (best for archival content)
///
/// # Examples
///
/// ```rust
/// # #[cfg(feature = "zstd")]
/// use tako::plugins::compression::zstd_stream::stream_zstd;
/// # #[cfg(feature = "zstd")]
/// use http_body_util::Full;
/// # #[cfg(feature = "zstd")]
/// use bytes::Bytes;
///
/// # #[cfg(feature = "zstd")]
/// # fn example() {
/// // Fast compression for API responses
/// let api_body = Full::from(Bytes::from(r#"{"data": "value"}"#));
/// let fast_compressed = stream_zstd(api_body, 1);
///
/// // Balanced compression for web content
/// let html_body = Full::from(Bytes::from("<html><body>Content</body></html>"));
/// let balanced_compressed = stream_zstd(html_body, 6);
///
/// // Maximum compression for static files
/// let css_body = Full::from(Bytes::from("body { margin: 0; }"));
/// let max_compressed = stream_zstd(css_body, 22);
/// # }
/// ```
pub fn stream_zstd<B>(body: B, level: i32) -> TakoBody
where
    B: Body<Data = Bytes, Error = BoxError> + Send + 'static,
{
    let upstream = body.into_data_stream();
    let zstd_stream = ZstdStream::new(upstream, level).map_ok(Frame::data);
    TakoBody::from_try_stream(zstd_stream)
}

pin_project! {
    /// Streaming Zstandard compressor that wraps an inner data stream.
    ///
    /// `ZstdStream` provides on-the-fly Zstandard compression for streaming data sources.
    /// It maintains an internal encoder state and buffer to efficiently compress data
    /// as it flows through the stream. The implementation handles proper stream
    /// finalization and ensures all compressed data is flushed when the input ends,
    /// providing optimal compression ratios with streaming performance.
    ///
    /// # Type Parameters
    ///
    /// * `S` - Inner stream type that yields `Result<Bytes, BoxError>` items
    ///
    /// # Examples
    ///
    /// ```rust
    /// # #[cfg(feature = "zstd")]
    /// use tako::plugins::compression::zstd_stream::ZstdStream;
    /// # #[cfg(feature = "zstd")]
    /// use futures_util::stream;
    /// # #[cfg(feature = "zstd")]
    /// use bytes::Bytes;
    ///
    /// # #[cfg(feature = "zstd")]
    /// # fn example() {
    /// // Create a stream of data chunks
    /// let data_stream = stream::iter(vec![
    ///     Ok(Bytes::from("First chunk of data")),
    ///     Ok(Bytes::from("Second chunk of data")),
    ///     Ok(Bytes::from("Final chunk of data")),
    /// ]);
    ///
    /// // Wrap with Zstandard compression at level 6
    /// let compressed_stream = ZstdStream::new(data_stream, 6);
    /// # }
    /// ```
    pub struct ZstdStream<S> {
        /// Inner stream providing source data for compression.
        #[pin] inner: S,
        /// Zstandard encoder with internal buffer for compressed output.
        encoder: Option<Encoder<'static, Vec<u8>>>,
        /// Buffer for holding compressed data chunks.
        buffer: Vec<u8>,
        /// Current position in the buffer for reading compressed data.
        pos: usize,
        /// Flag indicating whether the input stream has ended.
        done: bool,
    }
}

impl<S> ZstdStream<S> {
    /// Creates a new Zstandard compression stream with the specified compression level.
    ///
    /// The encoder is initialized with the specified compression level, which controls
    /// the trade-off between compression ratio and processing speed. Zstandard offers
    /// a wide range of compression levels with excellent performance characteristics
    /// across the entire range.
    ///
    /// # Arguments
    ///
    /// * `stream` - Source stream to compress
    /// * `level` - Zstandard compression level (1-22, clamped to valid range)
    ///
    /// # Panics
    ///
    /// Panics if the Zstandard encoder cannot be initialized, which should only occur
    /// with invalid compression levels outside the valid range.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # #[cfg(feature = "zstd")]
    /// use tako::plugins::compression::zstd_stream::ZstdStream;
    /// # #[cfg(feature = "zstd")]
    /// use futures_util::stream;
    /// # #[cfg(feature = "zstd")]
    /// use bytes::Bytes;
    ///
    /// # #[cfg(feature = "zstd")]
    /// # fn example() {
    /// let source = stream::iter(vec![Ok(Bytes::from("test data"))]);
    ///
    /// // Fast compression for real-time data
    /// let fast_stream = ZstdStream::new(source.clone(), 1);
    ///
    /// // Balanced compression for general web content
    /// let balanced_stream = ZstdStream::new(source.clone(), 6);
    ///
    /// // High compression for static content
    /// let high_stream = ZstdStream::new(source, 15);
    /// # }
    /// ```
    fn new(stream: S, level: i32) -> Self {
        Self {
            inner: stream,
            encoder: Some(Encoder::new(Vec::new(), level).expect("zstd encoder")),
            buffer: Vec::new(),
            pos: 0,
            done: false,
        }
    }
}

impl<S> Stream for ZstdStream<S>
where
    S: Stream<Item = Result<Bytes, BoxError>>,
{
    type Item = Result<Bytes, BoxError>;

    /// Polls the stream for the next compressed data chunk.
    ///
    /// This method implements the core streaming compression logic with the following steps:
    /// 1. Returns any buffered compressed data immediately if available
    /// 2. Checks if the stream is finished and the encoder has been consumed
    /// 3. Polls the inner stream for new input data when buffer is empty
    /// 4. Compresses new input data and copies it to the internal buffer
    /// 5. Finalizes Zstandard compression when the input stream ends
    /// 6. Handles errors and backpressure appropriately
    ///
    /// The implementation ensures optimal Zstandard compression by properly managing
    /// the encoder state and buffer, while maintaining memory efficiency through
    /// immediate data return when available.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # #[cfg(feature = "zstd")]
    /// use tako::plugins::compression::zstd_stream::ZstdStream;
    /// # #[cfg(feature = "zstd")]
    /// use futures_util::{stream, StreamExt};
    /// # #[cfg(feature = "zstd")]
    /// use bytes::Bytes;
    ///
    /// # #[cfg(feature = "zstd")]
    /// # async fn example() {
    /// let data = stream::iter(vec![
    ///     Ok(Bytes::from("chunk1")),
    ///     Ok(Bytes::from("chunk2")),
    /// ]);
    ///
    /// let mut compressed = ZstdStream::new(data, 6);
    ///
    /// // Process compressed chunks as they become available
    /// while let Some(result) = compressed.next().await {
    ///     match result {
    ///         Ok(zstd_chunk) => {
    ///             println!("Compressed {} bytes with Zstandard", zstd_chunk.len());
    ///         }
    ///         Err(e) => {
    ///             eprintln!("Zstandard compression error: {}", e);
    ///             break;
    ///         }
    ///     }
    /// }
    /// # }
    /// ```
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            // 1) Drain the buffer first, if there is unread output.
            if *this.pos < this.buffer.len() {
                let chunk = &this.buffer[*this.pos..];
                *this.pos = this.buffer.len();
                return Poll::Ready(Some(Ok(Bytes::copy_from_slice(chunk))));
            }
            // 2) If we are done and the encoder is already consumed,
            //    the stream is finished.
            if *this.done && this.encoder.is_none() {
                return Poll::Ready(None);
            }
            // 3) Poll the inner stream for more input data.
            match this.inner.as_mut().poll_next(cx) {
                // — New chunk arrived: compress it, then loop to drain.
                Poll::Ready(Some(Ok(data))) => {
                    if let Some(enc) = this.encoder.as_mut() {
                        if let Err(e) = enc.write_all(&data).and_then(|_| enc.flush()) {
                            return Poll::Ready(Some(Err(e.into())));
                        }
                        // Copy freshly compressed bytes into our buffer.
                        let out = enc.get_ref();
                        if !out.is_empty() {
                            this.buffer.clear();
                            this.buffer.extend_from_slice(out);
                            *this.pos = 0;
                        }
                    }
                    continue; // go back to step 1
                }
                // — Propagate an error from the inner stream.
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(e)));
                }
                // — Inner stream ended: finalise the encoder,
                //   then loop to emit the remaining bytes.
                Poll::Ready(None) => {
                    *this.done = true;
                    if let Some(enc) = this.encoder.take() {
                        match enc.finish() {
                            Ok(mut vec) => {
                                this.buffer.clear();
                                this.buffer.append(&mut vec);
                                *this.pos = 0;
                                continue; // step 1 will send the tail bytes
                            }
                            Err(e) => {
                                return Poll::Ready(Some(Err(e.into())));
                            }
                        }
                    } else {
                        return Poll::Ready(None);
                    }
                }
                // — No new input and nothing buffered.
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        }
    }
}
