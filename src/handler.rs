/// This module defines the `Handler` trait and the `BoxedHandler` struct, which are used to handle HTTP requests in a flexible and type-safe manner.
use std::{pin::Pin, sync::Arc};

use futures_util::future::BoxFuture;

use crate::{
    responder::Responder,
    types::{Request, Response},
};

/// The `Handler` trait represents an asynchronous function that processes an HTTP request and produces a response.
///
/// This trait is implemented for functions or closures that take a `Request` and return a future resolving to a `Response`.
///
/// # Example
///
/// ```rust
/// use tako::handler::Handler;
/// use tako::types::{Request, Response};
///
/// async fn my_handler(req: Request) -> Response {
///     // Process the request and return a response
///     Response::default()
/// }
///
/// // `my_handler` automatically implements the `Handler` trait.
/// ```
pub trait Handler: Send + Sync + 'static {
    type Future: Future<Output = Response> + Send + 'static;

    fn call(self, req: Request) -> Self::Future;
}

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

/// The `BoxedHandler` struct is a type-erased wrapper around a `Handler`.
///
/// This allows handlers to be stored and called dynamically, enabling greater flexibility in routing and middleware systems.
///
/// # Example
///
/// ```rust
/// use tako::handler::{BoxedHandler, Handler};
/// use tako::types::{Request, Response};
/// use std::sync::Arc;
///
/// async fn my_handler(req: Request) -> Response {
///     // Process the request and return a response
///     Response::default()
/// }
///
/// let handler = BoxedHandler::new(my_handler);
/// let response = handler.call(Request::default()).await;
/// ```
#[derive(Clone)]
pub struct BoxHandler {
    /// The inner function that processes the request and produces a response.
    inner: Arc<dyn Fn(Request) -> BoxFuture<'static, Response> + Send + Sync>,
}

impl BoxHandler {
    /// Creates a new `BoxedHandler` from a given `Handler`.
    ///
    /// # Arguments
    ///
    /// * `h` - A handler that implements the `Handler` trait.
    ///
    /// # Returns
    ///
    /// A `BoxedHandler` instance wrapping the provided handler.
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

    /// Calls the inner handler with the provided request.
    ///
    /// # Arguments
    ///
    /// * `req` - The HTTP request to be processed.
    ///
    /// # Returns
    ///
    /// A future that resolves to the HTTP response.
    pub(crate) fn call(&self, req: Request) -> BoxFuture<'_, Response> {
        (self.inner)(req)
    }
}
