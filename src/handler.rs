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
pub trait Handler: Send + Sync + 'static {
    /// Future type returned by the handler.
    type Future: Future<Output = Response> + Send + 'static;

    /// Calls the handler with the given request.
    fn call(self, req: Request) -> Self::Future;
}

/// Implements `Handler` for functions returning responder types.
///
/// This implementation enables any async function or closure that takes a `Request`
/// and returns a `Responder` to be used as a handler. The response is automatically
/// converted to the framework's standard `Response` type.
///
/// # Examples
///
/// ```rust
/// use tako::handler::Handler;
/// use tako::types::Request;
/// use http::StatusCode;
///
/// // Function handlers
/// async fn simple(_req: Request) -> &'static str {
///     "Simple response"
/// }
///
/// // Closure handlers
/// let closure_handler = |_req: Request| async {
///     (StatusCode::OK, "Closure response")
/// };
/// ```
impl<F, Fut, R> Handler for F
where
    F: FnOnce(Request) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = R> + Send,
    R: Responder,
{
    type Future = Pin<Box<dyn Future<Output = Response> + Send>>;

    fn call(self, req: Request) -> Self::Future {
        Box::pin(async move { (self)(req).await.into_response() })
    }
}

/// Type-erased handler wrapper for dynamic storage and composition.
#[derive(Clone)]
pub struct BoxHandler {
    /// The inner function that processes requests and produces responses.
    inner: Arc<dyn Fn(Request) -> BoxFuture<'static, Response> + Send + Sync>,
}

impl BoxHandler {
    /// Creates a new boxed handler from any handler implementation.
    pub(crate) fn new<H>(h: H) -> Self
    where
        H: Handler + Clone,
    {
        let inner = Arc::new(move |req: Request| {
            let handler = h.clone();
            Box::pin(async move { handler.call(req.into()).await }) as BoxFuture<'_, Response>
        });

        Self { inner }
    }

    /// Calls the boxed handler with the provided request.
    pub(crate) fn call(&self, req: Request) -> BoxFuture<'_, Response> {
        (self.inner)(req)
    }
}
