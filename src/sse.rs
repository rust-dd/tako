use std::convert::Infallible;

use bytes::{Bytes, BytesMut};
use http::header;
use http_body_util::StreamBody;
use tokio_stream::{Stream, StreamExt};

use crate::{body::TakoBody, bytes::TakoBytes, responder::Responder, types::Response};

const PREFIX: &[u8] = b"data: ";
const SUFFIX: &[u8] = b"\n\n";

const fn ps_len() -> usize {
    PREFIX.len() + SUFFIX.len()
}

pub struct Sse<S>
where
    S: Stream<Item = TakoBytes> + Send + 'static,
{
    pub stream: S,
}

impl<S> Sse<S>
where
    S: Stream<Item = TakoBytes> + Send + 'static,
{
    pub fn new(stream: S) -> Self {
        Self { stream }
    }
}

impl<S> Responder for Sse<S>
where
    S: Stream<Item = TakoBytes> + Send + 'static,
{
    fn into_response(self) -> Response {
        let stream = self.stream.map(|TakoBytes(msg)| {
            let mut buf = BytesMut::with_capacity(ps_len() + msg.len());
            buf.extend_from_slice(PREFIX);
            buf.extend_from_slice(&msg);
            buf.extend_from_slice(SUFFIX);
            Ok::<_, Infallible>(hyper::body::Frame::data(Bytes::from(buf)))
        });

        http::Response::builder()
            .status(200)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .header(header::CONNECTION, "keep-alive")
            .body(TakoBody::new(StreamBody::new(stream)))
            .unwrap()
    }
}
