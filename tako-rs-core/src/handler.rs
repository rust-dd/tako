#![allow(non_snake_case)]

//! Request handler traits and implementations for type-safe HTTP processing.
//!
//! This module provides the core handler abstraction for Tako applications. Handlers are
//! asynchronous functions that process HTTP requests and produce responses. The `Handler`
//! trait enables type-safe request processing with automatic response conversion, while
//! `BoxHandler` provides type erasure for dynamic handler storage and composition.
//!
//! # Examples
//!
//! ```rust
//! use tako::router::Router;
//! use tako::types::{Request, Response};
//! use tako::body::TakoBody;
//! use std::future::Future;
//!
//! // Simple handler function
//! async fn hello_handler(_req: Request) -> &'static str {
//!     "Hello, World!"
//! }
//!
//! // Handler with custom response type
//! async fn json_handler(_req: Request) -> Response {
//!     Response::new(TakoBody::from(r#"{"message": "Hello, JSON!"}"#))
//! }
//!
//! let mut router = Router::new();
//! router.get("/", hello_handler);
//! ```

mod future;

use std::future::Future;
use std::sync::Arc;

pub(crate) use self::future::HandlerFuture;
use crate::extractors::Entries;
use crate::extractors::FromRequest;
use crate::extractors::FromRequestParts;
use crate::responder::Responder;
use crate::types::Request;
use crate::types::Response;

/// Trait for asynchronous HTTP request handlers.
///
/// The `Handler` trait represents functions that process HTTP requests and produce responses.
/// It is automatically implemented for async functions and closures that take a `Request`
/// and return any type implementing `Responder`. This enables flexible handler composition
/// and type-safe response generation throughout the framework.
///
/// # Examples
///
/// ```rust
/// use tako_rs_core::handler::Handler;
/// use tako::types::{Request, Response};
/// use tako::responder::Responder;
/// use http::StatusCode;
///
/// // Simple string handler
/// async fn text_handler(_req: Request) -> &'static str {
///     "Hello, World!"
/// }
///
/// // Status code with body
/// async fn status_handler(_req: Request) -> (StatusCode, &'static str) {
///     (StatusCode::CREATED, "Resource created")
/// }
///
/// // Custom response handler
/// async fn custom_handler(_req: Request) -> Response {
///     Response::new(tako::body::TakoBody::from("Custom response"))
/// }
/// ```
/// Body extractors must be last; all preceding arguments implement `FromRequestParts`.
///
/// ```compile_fail
/// use tako_rs_core::{router::Router, extractors::json::Json};
/// async fn two_bodies(_: Json<String>, _: Json<String>) {}
/// Router::new().post("/", two_bodies);
/// ```
pub trait Handler<T>: Send + Sync + 'static {
  /// Framework entries the handler's extractors read.
  const ENTRIES: Entries = Entries::ALL;

  /// Calls the handler with the given request.
  ///
  /// Returns an unboxed future; `BoxHandler` erases its type, without a heap
  /// allocation when the future is small.
  fn call(self, req: Request) -> impl Future<Output = Response> + Send + 'static;
}

/// Type-erased handler wrapper for dynamic storage and composition.
///
/// Handlers can now be written with or without extractor parameters, similar to Axum.
/// For example: `async fn handler() -> impl Responder`, `async fn handler(Json<T>) -> _`,
/// or `async fn handler(Path(p): Path<Params>, Query(q): Query<Search>) -> _`.
#[derive(Clone)]
pub struct BoxHandler {
  /// The inner function that processes requests and produces responses.
  inner: Arc<dyn Fn(Request) -> HandlerFuture + Send + Sync>,
  entries: Entries,
}

impl BoxHandler {
  /// Creates a new boxed handler from any handler implementation.
  ///
  /// This is the single type-erasure point: `Handler::call` returns an unboxed
  /// future, which [`HandlerFuture`] stores inline or boxes.
  pub(crate) fn new<H, T>(h: H) -> Self
  where
    H: Handler<T> + Clone,
  {
    let inner = Arc::new(move |req: Request| HandlerFuture::new(h.clone().call(req)));

    Self {
      inner,
      entries: H::ENTRIES,
    }
  }

  /// Framework entries the handler's extractors read.
  pub(crate) fn entries(&self) -> Entries {
    self.entries
  }

  /// Calls the boxed handler with the provided request.
  pub(crate) fn call(&self, req: Request) -> HandlerFuture {
    (self.inner)(req)
  }
}

impl<F, Fut, R> Handler<()> for F
where
  F: FnOnce() -> Fut + Clone + Send + Sync + 'static,
  Fut: Future<Output = R> + Send + 'static,
  R: Responder,
{
  const ENTRIES: Entries = Entries::NONE;

  fn call(self, req: Request) -> impl Future<Output = Response> + Send + 'static {
    crate::recycle::recycle_parts(req.into_parts().0);
    async move { (self)().await.into_response() }
  }
}

trait Extract: Sized + Send {
  type Error: Responder;
  const ENTRIES: Entries;
  fn extract(req: &mut Request) -> impl Future<Output = Result<Self, Self::Error>> + Send;
}

impl<T, E> Extract for T
where
  T: Send,
  E: Responder,
  for<'a> T: FromRequest<'a, Error = E>,
{
  type Error = E;
  const ENTRIES: Entries = <T as FromRequest<'static>>::ENTRIES;
  fn extract(req: &mut Request) -> impl Future<Output = Result<Self, E>> + Send {
    T::from_request(req)
  }
}

trait ExtractParts: Sized + Send {
  type Error: Responder;
  const ENTRIES: Entries;
  fn extract(
    parts: &mut http::request::Parts,
  ) -> impl Future<Output = Result<Self, Self::Error>> + Send;
}

impl<T, E> ExtractParts for T
where
  T: Send,
  E: Responder,
  for<'a> T: FromRequestParts<'a, Error = E>,
{
  type Error = E;
  const ENTRIES: Entries = <T as FromRequestParts<'static>>::ENTRIES;
  fn extract(parts: &mut http::request::Parts) -> impl Future<Output = Result<Self, E>> + Send {
    T::from_request_parts(parts)
  }
}

macro_rules! impl_handler {
  ($($Part:ident,)* ; $Last:ident) => {
    impl<Func, Fut, R, $($Part,)* $Last> Handler<($($Part,)* $Last,)> for Func
    where
      Func: FnOnce($($Part,)* $Last) -> Fut + Clone + Send + Sync + 'static,
      Fut: Future<Output = R> + Send + 'static,
      R: Responder,
      $($Part: ExtractParts + 'static,)*
      $Last: Extract + 'static,
    {
      const ENTRIES: Entries = Entries::NONE
        $(.union(<$Part as ExtractParts>::ENTRIES))*
        .union(<$Last as Extract>::ENTRIES);

      fn call(self, req: Request) -> impl Future<Output = Response> + Send + 'static {
        async move {
          #[allow(unused_mut)]
          let (mut parts, body) = req.into_parts();
          $(let $Part = match <$Part as ExtractParts>::extract(&mut parts).await {
            Ok(value) => value,
            Err(error) => return error.into_response(),
          };)*
          let mut req = Request::from_parts(parts, body);
          let last = match <$Last as Extract>::extract(&mut req).await {
            Ok(value) => value,
            Err(error) => return error.into_response(),
          };
          crate::recycle::recycle_headers(std::mem::take(req.headers_mut()));
          crate::recycle::recycle_extensions(std::mem::take(req.extensions_mut()));
          (self)($($Part,)* last).await.into_response()
        }
      }
    }
  };
}

impl_handler!(; T1);
impl_handler!(T1, ; T2);
impl_handler!(T1, T2, ; T3);
impl_handler!(T1, T2, T3, ; T4);
impl_handler!(T1, T2, T3, T4, ; T5);
impl_handler!(T1, T2, T3, T4, T5, ; T6);
impl_handler!(T1, T2, T3, T4, T5, T6, ; T7);
impl_handler!(T1, T2, T3, T4, T5, T6, T7, ; T8);
impl_handler!(T1, T2, T3, T4, T5, T6, T7, T8, ; T9);
impl_handler!(T1, T2, T3, T4, T5, T6, T7, T8, T9, ; T10);
impl_handler!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, ; T11);
impl_handler!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, ; T12);

#[cfg(test)]
mod tests {
  use std::collections::HashMap;
  use std::convert::Infallible;
  use std::future::Future;

  use http::HeaderMap;
  use http::HeaderValue;
  use http::header::HOST;

  use super::BoxHandler;
  use crate::body::TakoBody;
  use crate::extractors::Entries;
  use crate::extractors::FromRequest;
  use crate::extractors::json::Json;
  use crate::extractors::params::Params;
  use crate::router_state::MatchedPath;
  use crate::types::Request;

  struct Custom;

  impl<'a> FromRequest<'a> for Custom {
    type Error = Infallible;

    fn from_request(
      _req: &'a mut Request,
    ) -> impl Future<Output = Result<Self, Self::Error>> + Send + 'a {
      std::future::ready(Ok(Custom))
    }
  }

  type Ids = Params<HashMap<String, String>>;

  #[test]
  fn handlers_need_the_entries_their_extractors_list() {
    assert_eq!(
      BoxHandler::new::<_, ()>(|| async { "ok" }).entries(),
      Entries::NONE
    );
    let path = BoxHandler::new::<_, (MatchedPath,)>(|_: MatchedPath| async { "ok" });
    assert_eq!(path.entries(), Entries::MATCHED_PATH);
    let both = BoxHandler::new::<_, (Ids, Json<serde_json::Value>)>(
      |_: Ids, _: Json<serde_json::Value>| async { "ok" },
    );
    assert_eq!(
      both.entries(),
      Entries::PARAMS
        .union(Entries::BODY_LIMIT)
        .union(Entries::SIMD_JSON)
    );
  }

  #[test]
  fn raw_requests_and_unannotated_extractors_need_every_entry() {
    let raw = BoxHandler::new::<_, (Request,)>(|_: Request| async { "ok" });
    assert_eq!(raw.entries(), Entries::ALL);
    let custom = BoxHandler::new::<_, (Custom,)>(|_: Custom| async { "ok" });
    assert_eq!(custom.entries(), Entries::ALL);
  }

  fn request_with_headers(capacity: usize) -> (Request, usize) {
    let mut headers = HeaderMap::with_capacity(capacity);
    headers.insert(HOST, HeaderValue::from_static("example.com"));
    let capacity = headers.capacity();
    let mut req = Request::new(TakoBody::empty());
    *req.headers_mut() = headers;
    (req, capacity)
  }

  #[tokio::test]
  async fn handler_without_arguments_answers_with_the_request_header_map() {
    let handler = BoxHandler::new::<_, ()>(|| async { "ok" });
    let (req, capacity) = request_with_headers(64);
    let resp = handler.call(req).await;
    assert_eq!(resp.headers().capacity(), capacity);
    assert!(!resp.headers().contains_key(HOST));
  }

  #[tokio::test]
  async fn handler_with_extractors_answers_with_the_request_header_map() {
    let handler =
      BoxHandler::new::<_, (MatchedPath,)>(
        |path: MatchedPath| async move { path.as_str().to_owned() },
      );
    let (mut req, capacity) = request_with_headers(64);
    req
      .extensions_mut()
      .insert(MatchedPath("/users/{id}".into()));
    let resp = handler.call(req).await;
    assert_eq!(resp.headers().capacity(), capacity);
    assert!(!resp.headers().contains_key(HOST));
  }
}
