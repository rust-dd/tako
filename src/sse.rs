use std::convert::Infallible;

use bytes::Bytes;
use http::header;
use http_body_util::StreamBody;
use tokio_stream::{Stream, StreamExt};

use crate::{body::TakoBody, responder::Responder, types::Response};

pub struct SseString<S>
where
    S: Stream<Item = String> + Send + 'static,
{
    pub stream: S,
}

impl<S> Responder for SseString<S>
where
    S: Stream<Item = String> + Send + 'static,
{
    fn into_response(self) -> Response {
        let stream = self.stream.map(|msg| {
            let bytes = Bytes::from(format!("data: {}\n\n", msg));
            Ok::<_, Infallible>(hyper::body::Frame::data(bytes))
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

pub struct SseBytes<S>
where
    S: Stream<Item = Bytes> + Send + 'static,
{
    pub stream: S,
}

impl<S> Responder for SseBytes<S>
where
    S: Stream<Item = Bytes> + Send + 'static,
{
    fn into_response(self) -> Response {
        let stream = self
            .stream
            .map(|msg| Ok::<_, Infallible>(hyper::body::Frame::data(msg)));

        http::Response::builder()
            .status(200)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .header(header::CONNECTION, "keep-alive")
            .body(TakoBody::new(StreamBody::new(stream)))
            .unwrap()
    }
}
