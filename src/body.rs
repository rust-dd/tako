use std::{
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;

use http_body_util::{BodyExt, Empty};
use hyper::body::{Body, Frame, SizeHint};

use crate::types::{BoxBody, BoxError};

pub struct TakoBody(BoxBody);

impl TakoBody {
    pub fn new<B>(body: B) -> Self
    where
        B: Body<Data = Bytes> + Send + 'static,
        B::Error: Into<BoxError>,
    {
        Self(body.map_err(|e| e.into()).boxed_unsync())
    }

    pub fn empty() -> Self {
        Self::new(Empty::new())
    }
}

impl Default for TakoBody {
    fn default() -> Self {
        Self::empty()
    }
}

impl Body for TakoBody {
    type Data = Bytes;
    type Error = BoxError;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut self.0).poll_frame(cx)
    }

    fn size_hint(&self) -> SizeHint {
        self.0.size_hint()
    }

    fn is_end_stream(&self) -> bool {
        self.0.is_end_stream()
    }
}
