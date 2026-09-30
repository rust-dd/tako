//! Streaming gRPC: the server-streaming responder, the client-streaming
//! extractor, and the bidirectional extractor that pairs the two.
//!
//! A reply starts as soon as the handler returns, while the request body is
//! still open, so a bidirectional handler reads and writes concurrently.

use std::convert::Infallible;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use bytes::Bytes;
use bytes::BytesMut;
use futures_util::Stream;
use http::HeaderMap;
use http_body::Body;
use http_body::Frame;
use prost::Message;

use super::GrpcError;
use super::framing::FrameDecoder;
use super::framing::encode_into;
use super::message::is_grpc_content_type;
use super::status::GrpcStatus;
use super::timeout::DeadlineTimer;
use super::timeout::GrpcDeadline;
use crate::body::TakoBody;
use crate::extractors::FromRequest;
use crate::responder::Responder;
use crate::types::Request;
use crate::types::Response;

/// Server-streaming gRPC response.
///
/// Sends each `Ok` item as a length-prefixed message; items that are ready
/// together share one DATA frame. The call ends with
/// `grpc-status: 0` when the stream finishes, with the status of the first
/// `Err` item (later items are dropped), or with `DeadlineExceeded` once the
/// optional deadline passes. A client that cancels drops the stream.
pub struct GrpcServerStream<S, T>
where
  S: Stream<Item = Result<T, GrpcStatus>> + Send + 'static,
  T: Message + Send + 'static,
{
  /// Messages to send; the first `Err` ends the call with its status.
  pub stream: S,
  /// Server metadata sent as response headers (initial metadata).
  pub initial_metadata: HeaderMap,
  /// Ends the call with `DeadlineExceeded` once reached.
  pub deadline: Option<GrpcDeadline>,
}

impl<S, T> GrpcServerStream<S, T>
where
  S: Stream<Item = Result<T, GrpcStatus>> + Send + 'static,
  T: Message + Send + 'static,
{
  pub fn new(stream: S) -> Self {
    Self {
      stream,
      initial_metadata: HeaderMap::new(),
      deadline: None,
    }
  }

  pub fn with_metadata(mut self, headers: HeaderMap) -> Self {
    self.initial_metadata = headers;
    self
  }

  /// Ends the call with `DeadlineExceeded` at `deadline`, typically the
  /// client's `grpc-timeout` taken as an `Option<GrpcDeadline>` extractor.
  pub fn with_deadline(mut self, deadline: impl Into<Option<GrpcDeadline>>) -> Self {
    self.deadline = deadline.into();
    self
  }
}

impl<S, T> Responder for GrpcServerStream<S, T>
where
  S: Stream<Item = Result<T, GrpcStatus>> + Send + 'static,
  T: Message + Send + 'static,
{
  fn into_response(self) -> Response {
    let body = GrpcStreamBody::new(self.stream, self.deadline);
    let mut resp = Response::new(TakoBody::new(body));
    let headers = resp.headers_mut();
    headers.insert(
      http::header::CONTENT_TYPE,
      http::HeaderValue::from_static("application/grpc"),
    );
    headers.extend(self.initial_metadata);
    resp
  }
}

/// Messages that are ready together share one DATA frame up to this size, as
/// in tonic. A frame per tiny message wastes framing overhead, and `h2`
/// clients close the connection when too many small frames wait unread.
pub(super) const BATCH_LIMIT: usize = 32 * 1024;

/// Response body of a [`GrpcServerStream`]: DATA frames carrying the
/// messages, then exactly one trailers frame carrying `grpc-status`.
struct GrpcStreamBody<S> {
  /// `None` once the call has ended.
  stream: Option<Pin<Box<S>>>,
  deadline: Option<DeadlineTimer>,
  buffer: BytesMut,
  /// Final status, sent after the messages already batched.
  status: Option<GrpcStatus>,
}

impl<S> GrpcStreamBody<S> {
  fn new(stream: S, deadline: Option<GrpcDeadline>) -> Self {
    Self {
      stream: Some(Box::pin(stream)),
      deadline: deadline.map(DeadlineTimer::new),
      buffer: BytesMut::new(),
      status: None,
    }
  }

  fn end(&mut self, status: GrpcStatus) {
    self.stream = None;
    self.deadline = None;
    self.status = Some(status);
  }
}

impl<S, T> Body for GrpcStreamBody<S>
where
  S: Stream<Item = Result<T, GrpcStatus>>,
  T: Message,
{
  type Data = Bytes;
  type Error = Infallible;

  fn poll_frame(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
    let this = self.get_mut();
    if let Some(deadline) = this.deadline.as_mut()
      && deadline.poll_elapsed(cx).is_ready()
    {
      this.end(GrpcStatus::deadline_exceeded());
    }
    while let Some(stream) = this.stream.as_mut()
      && this.buffer.len() < BATCH_LIMIT
    {
      match stream.as_mut().poll_next(cx) {
        Poll::Ready(Some(Ok(message))) => encode_into(&message, &mut this.buffer),
        Poll::Ready(Some(Err(status))) => this.end(status),
        Poll::Ready(None) => this.end(GrpcStatus::ok()),
        Poll::Pending => break,
      }
    }
    if !this.buffer.is_empty() {
      return Poll::Ready(Some(Ok(Frame::data(this.buffer.split().freeze()))));
    }
    match this.status.take() {
      Some(status) => Poll::Ready(Some(Ok(Frame::trailers(status.write_trailers())))),
      None if this.stream.is_none() => Poll::Ready(None),
      None => Poll::Pending,
    }
  }

  fn is_end_stream(&self) -> bool {
    self.stream.is_none() && self.status.is_none() && self.buffer.is_empty()
  }
}

/// Client-streaming gRPC extractor.
///
/// Yields the request's messages as they arrive. The stream ends after the
/// first error; a request that stops inside a message yields
/// [`GrpcError::InvalidFrame`]. Each message is capped at
/// [`MAX_GRPC_MESSAGE_SIZE`](super::MAX_GRPC_MESSAGE_SIZE).
pub struct GrpcClientStream<T: Message + Default + Send + 'static> {
  pub stream: Pin<Box<dyn Stream<Item = Result<T, GrpcError>> + Send>>,
}

impl<'a, T> FromRequest<'a> for GrpcClientStream<T>
where
  T: Message + Default + Send + 'static,
{
  type Error = GrpcError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    let stream = is_grpc_content_type(req).then(|| GrpcClientStream {
      stream: Box::pin(FrameDecoder::<T>::new(std::mem::take(req.body_mut()))),
    });
    futures_util::future::ready(stream.ok_or(GrpcError::InvalidContentType))
  }
}

impl<T: Message + Default + Send + 'static> Stream for GrpcClientStream<T> {
  type Item = Result<T, GrpcError>;

  fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    self.get_mut().stream.as_mut().poll_next(cx)
  }
}

/// Bidirectional gRPC extractor.
///
/// Reads `Req` messages like a [`GrpcClientStream`] and names the `Resp`
/// type of the reply, so [`respond`](Self::respond) can build the
/// [`GrpcServerStream`] with full type inference.
pub struct GrpcBidi<Req, Resp>
where
  Req: Message + Default + Send + 'static,
  Resp: Message + Send + 'static,
{
  pub inbound: GrpcClientStream<Req>,
  pub _phantom: PhantomData<Resp>,
}

impl<Req, Resp> GrpcBidi<Req, Resp>
where
  Req: Message + Default + Send + 'static,
  Resp: Message + Send + 'static,
{
  /// Builds the reply from the inbound messages; it is sent while they are
  /// still arriving.
  pub fn respond<S>(
    self,
    reply: impl FnOnce(GrpcClientStream<Req>) -> S,
  ) -> GrpcServerStream<S, Resp>
  where
    S: Stream<Item = Result<Resp, GrpcStatus>> + Send + 'static,
  {
    GrpcServerStream::new(reply(self.inbound))
  }
}

impl<'a, Req, Resp> FromRequest<'a> for GrpcBidi<Req, Resp>
where
  Req: Message + Default + Send + 'static,
  Resp: Message + Send + 'static,
{
  type Error = GrpcError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      Ok(GrpcBidi {
        inbound: GrpcClientStream::<Req>::from_request(req).await?,
        _phantom: PhantomData,
      })
    }
  }
}

impl<Req, Resp> Stream for GrpcBidi<Req, Resp>
where
  Req: Message + Default + Send + 'static,
  Resp: Message + Send + Unpin + 'static,
{
  type Item = Result<Req, GrpcError>;

  fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    Pin::new(&mut self.get_mut().inbound).poll_next(cx)
  }
}
