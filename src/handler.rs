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
//! use tako::handler::{Handler, BoxHandler};
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
//! // Box handlers for dynamic storage
//! let boxed = BoxHandler::new(hello_handler);
//! ```

use std::{future::Future, pin::Pin, sync::Arc};

use futures_util::future::BoxFuture;

use crate::{
  extractors::FromRequest,
  responder::Responder,
  types::{Request, Response},
};

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
/// use tako::handler::Handler;
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
pub trait Handler<T>: Send + Sync + 'static {
  /// Future type returned by the handler.
  type Future: Future<Output = Response> + Send + 'static;

  /// Calls the handler with the given request.
  fn call(self, req: Request) -> Self::Future;
}

/// Implements `Handler` for functions returning responder types using extractor arguments.
///
/// Handlers can now be written with or without extractor parameters, similar to Axum.
/// For example: `async fn handler() -> impl Responder`, `async fn handler(Json<T>) -> _`,
/// or `async fn handler(Path(p): Path<'_>, Query<Q>) -> _`.

/// Type-erased handler wrapper for dynamic storage and composition.
#[derive(Clone)]
pub struct BoxHandler {
  /// The inner function that processes requests and produces responses.
  inner: Arc<dyn Fn(Request) -> BoxFuture<'static, Response> + Send + Sync>,
}

impl BoxHandler {
  /// Creates a new boxed handler from any handler implementation.
  pub(crate) fn new<H, T>(h: H) -> Self
  where
    H: Handler<T> + Clone,
  {
    let inner = Arc::new(move |req: Request| {
      let handler = h.clone();
      Box::pin(async move { handler.call(req).await }) as BoxFuture<'_, Response>
    });

    Self { inner }
  }

  /// Calls the boxed handler with the provided request.
  pub(crate) fn call(&self, req: Request) -> BoxFuture<'_, Response> {
    (self.inner)(req)
  }
}

// Zero-argument handlers: `async fn handler() -> impl Responder`
impl<F, Fut, R> Handler<()> for F
where
  F: FnOnce() -> Fut + Clone + Send + Sync + 'static,
  Fut: Future<Output = R> + Send + 'static,
  R: Responder,
{
  type Future = Pin<Box<dyn Future<Output = Response> + Send>>;

  fn call(self, _req: Request) -> Self::Future {
    Box::pin(async move { (self)().await.into_response() })
  }
}

// Back-compat: single Request arg handlers: `async fn handler(req: Request) -> impl Responder`
impl<F, Fut, R> Handler<(Request,)> for F
where
  F: FnOnce(Request) -> Fut + Clone + Send + Sync + 'static,
  Fut: Future<Output = R> + Send + 'static,
  R: Responder,
{
  type Future = Pin<Box<dyn Future<Output = Response> + Send>>;

  fn call(self, req: Request) -> Self::Future {
    Box::pin(async move { (self)(req).await.into_response() })
  }
}

// Abstraction over extraction that avoids HRTB bounds in impls.
trait Extract: Sized + Send {
  type Error: Responder;

  fn extract<'a>(
    req: &'a mut Request,
  ) -> Pin<Box<dyn Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a>>;
}

impl<T, E> Extract for T
where
  T: Send,
  E: Responder,
  for<'a> T: FromRequest<'a, Error = E>,
{
  type Error = E;

  fn extract<'a>(
    req: &'a mut Request,
  ) -> Pin<Box<dyn Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a>> {
    Box::pin(<T as FromRequest<'a>>::from_request(req))
  }
}

macro_rules! impl_handler {
    ($($T:ident),+ $(,)?) => {
        impl<Func, Fut, R, $($T,)*> Handler<($($T,)*)> for Func
        where
            Func: FnOnce($($T),*) -> Fut + Clone + Send + Sync + 'static,
            Fut: Future<Output = R> + Send + 'static,
            R: Responder,
            $( $T: Extract + Send, )*
        {
            type Future = Pin<Box<dyn Future<Output = Response> + Send>>;

            fn call(self, mut req: Request) -> Self::Future {
                Box::pin(async move {
                    $(
                        let $T = match <$T as Extract>::extract(&mut req).await {
                            Ok(value) => value,
                            Err(err) => {
                                return err.into_response();
                            }
                        };
                    )*
                    (self)($($T),*).await.into_response()
                })
            }
        }
    };
}

impl_handler!(T1);
impl_handler!(T1, T2);
impl_handler!(T1, T2, T3);
impl_handler!(T1, T2, T3, T4);
impl_handler!(T1, T2, T3, T4, T5);
impl_handler!(T1, T2, T3, T4, T5, T6);
impl_handler!(T1, T2, T3, T4, T5, T6, T7);
impl_handler!(T1, T2, T3, T4, T5, T6, T7, T8);
impl_handler!(T1, T2, T3, T4, T5, T6, T7, T8, T9);
impl_handler!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
impl_handler!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);
impl_handler!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12);
