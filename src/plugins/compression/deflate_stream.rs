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

pub fn stream_deflate<B>(body: B, level: u32) -> TakoBody
where
    B: Body<Data = Bytes, Error = BoxError> + Send + 'static,
{
    let upstream = body.into_data_stream();
    let deflate = DeflateStream::new(upstream, level).map_ok(Frame::data);
    TakoBody::from_try_stream(deflate)
}

pin_project! {
    pub struct DeflateStream<S> {
        #[pin] inner: S,
        encoder: DeflateEncoder<Vec<u8>>,
        pos: usize,
        done: bool,
    }
}

impl<S> DeflateStream<S> {
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

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            if *this.pos < this.encoder.get_ref().len() {
                let buf = &this.encoder.get_ref()[*this.pos..];
                *this.pos = this.encoder.get_ref().len();
                return Poll::Ready(Some(Ok(Bytes::copy_from_slice(buf))));
            }

            if *this.done {
                return Poll::Ready(None);
            }

            match this.inner.as_mut().poll_next(cx) {
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
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(e)));
                }
                Poll::Ready(None) => {
                    *this.done = true;
                    if let Err(e) = this.encoder.try_finish() {
                        return Poll::Ready(Some(Err(e.into())));
                    }
                    continue;
                }
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        }
    }
}
