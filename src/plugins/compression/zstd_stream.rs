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
    pub struct ZstdStream<S> {
        #[pin] inner: S,
        encoder: Option<Encoder<'static, Vec<u8>>>,
        buffer: Vec<u8>,
        pos: usize,
        done: bool,
    }
}

impl<S> ZstdStream<S> {
    /// Creates a new Zstandard compression stream with the specified compression level.
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
