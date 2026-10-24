use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::Context;
use std::task::Poll;

use bytes::Bytes;
use futures_util::Stream;
use http::HeaderMap;
use http_body::Body;
use http_body::Frame;
use http_body_util::BodyExt;
use pin_project_lite::pin_project;
use tako_rs_core::body::TakoBody;
use tako_rs_core::types::BoxError;

type Trailers = Arc<Mutex<Option<HeaderMap>>>;

pub(super) fn encode<B, S>(body: B, encoder: impl FnOnce(DataFrames<B>) -> S) -> TakoBody
where
  B: Body<Data = Bytes, Error = BoxError> + Send + 'static,
  S: Stream<Item = Result<Bytes, BoxError>> + Send + 'static,
{
  let trailers = Arc::new(Mutex::new(None));
  let input = DataFrames {
    body,
    trailers: trailers.clone(),
  };
  TakoBody::from_try_stream(EncodedFrames {
    encoded: encoder(input),
    trailers,
    done: false,
  })
}

pub(super) fn buffered(body: Bytes, trailers: Option<HeaderMap>) -> TakoBody {
  let body = TakoBody::from(body);
  match trailers {
    Some(trailers) => TakoBody::new(body.with_trailers(std::future::ready(Some(Ok(trailers))))),
    None => body,
  }
}

pin_project! {
  pub(super) struct DataFrames<B> {
    #[pin]
    body: B,
    trailers: Trailers,
  }
}

impl<B: Body<Data = Bytes, Error = BoxError>> Stream for DataFrames<B> {
  type Item = Result<Bytes, BoxError>;

  fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    let mut this = self.project();
    loop {
      match std::task::ready!(this.body.as_mut().poll_frame(cx)) {
        Some(Ok(frame)) => match frame.into_data() {
          Ok(data) => return Poll::Ready(Some(Ok(data))),
          Err(frame) => {
            if let Ok(trailers) = frame.into_trailers() {
              let mut saved = this.trailers.lock().expect("trailer lock poisoned");
              saved.get_or_insert_with(HeaderMap::new).extend(trailers);
            }
          }
        },
        Some(Err(error)) => return Poll::Ready(Some(Err(error))),
        None => return Poll::Ready(None),
      }
    }
  }
}

pin_project! {
  struct EncodedFrames<S> {
    #[pin]
    encoded: S,
    trailers: Trailers,
    done: bool,
  }
}

impl<S: Stream<Item = Result<Bytes, BoxError>>> Stream for EncodedFrames<S> {
  type Item = Result<Frame<Bytes>, BoxError>;

  fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    let this = self.project();
    if *this.done {
      return Poll::Ready(None);
    }
    match std::task::ready!(this.encoded.poll_next(cx)) {
      Some(Ok(bytes)) => Poll::Ready(Some(Ok(Frame::data(bytes)))),
      Some(Err(error)) => {
        *this.done = true;
        this.trailers.lock().expect("trailer lock poisoned").take();
        Poll::Ready(Some(Err(error)))
      }
      None => {
        *this.done = true;
        let trailers = this.trailers.lock().expect("trailer lock poisoned").take();
        Poll::Ready(trailers.map(|trailers| Ok(Frame::trailers(trailers))))
      }
    }
  }
}
