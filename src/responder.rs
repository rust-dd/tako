use std::convert::Infallible;

use bytes::Bytes;
use http::Response;
use http_body_util::Full;

use crate::body::TakoBody;

pub trait Responder {
    fn into_response(self) -> Response<TakoBody>;
}

impl Responder for Response<TakoBody> {
    fn into_response(self) -> Response<TakoBody> {
        self
    }
}

impl Responder for &'static str {
    fn into_response(self) -> Response<TakoBody> {
        Response::new(TakoBody::new(Full::from(Bytes::from_static(
            self.as_bytes(),
        ))))
    }
}

impl Responder for String {
    fn into_response(self) -> Response<TakoBody> {
        Response::new(TakoBody::new(Full::from(Bytes::from(self))))
    }
}

impl Responder for () {
    fn into_response(self) -> Response<TakoBody> {
        Response::new(TakoBody::empty())
    }
}

impl Responder for Infallible {
    fn into_response(self) -> Response<TakoBody> {
        match self {}
    }
}
