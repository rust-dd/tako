#![allow(non_snake_case)]

//! `!Send` request handlers.
//!
//! Mirrors [`tako_core::handler::Handler`] without the `Send + Sync` bounds.
//! A blanket implementation makes every existing thread-safe handler also a
//! [`LocalHandler`], so user code that already works with the standard
//! [`tako::router::Router`] keeps compiling against [`crate::router::LocalRouter`].

use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use tako_core::extractors::FromRequest;
use tako_core::responder::Responder;
use tako_core::types::Request;
use tako_core::types::Response;

/// Future trait alias for `!Send` handler/middleware return values.
pub type LocalBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// Trait for `!Send` HTTP request handlers.
pub trait LocalHandler<T>: 'static {
  /// Calls the handler and returns a future that resolves to a response.
  fn call(self, req: Request) -> impl Future<Output = Response> + 'static;
}

/// Type-erased `!Send` handler wrapper.
#[derive(Clone)]
pub struct LocalBoxHandler {
  inner: Rc<dyn Fn(Request) -> LocalBoxFuture<'static, Response>>,
}

impl LocalBoxHandler {
  pub fn new<H, T>(h: H) -> Self
  where
    H: LocalHandler<T> + Clone,
  {
    let inner: Rc<dyn Fn(Request) -> LocalBoxFuture<'static, Response>> =
      Rc::new(move |req: Request| -> LocalBoxFuture<'static, Response> {
        Box::pin(h.clone().call(req))
      });
    Self { inner }
  }

  pub fn call(&self, req: Request) -> LocalBoxFuture<'_, Response> {
    (self.inner)(req)
  }
}

// Zero-argument: async fn() -> impl Responder
impl<F, Fut, R> LocalHandler<()> for F
where
  F: FnOnce() -> Fut + Clone + 'static,
  Fut: Future<Output = R> + 'static,
  R: Responder,
{
  fn call(self, _req: Request) -> impl Future<Output = Response> + 'static {
    async move { (self)().await.into_response() }
  }
}

// Note: a `LocalHandler<(Request,)>` impl for raw-request handlers conflicts with
// the n-arg extractor macro under cross-crate coherence (the compiler can't prove
// `Request: !FromRequest` will hold forever). For a `!Send` raw-request handler
// in `LocalRouter`, take a `Request` extension or wrap with a custom extractor.

trait Extract: Sized {
  type Error: Responder;
  fn extract<'a>(
    req: &'a mut Request,
  ) -> Pin<Box<dyn Future<Output = core::result::Result<Self, Self::Error>> + 'a>>;
}

impl<T, E> Extract for T
where
  E: Responder,
  for<'a> T: FromRequest<'a, Error = E>,
{
  type Error = E;
  fn extract<'a>(
    req: &'a mut Request,
  ) -> Pin<Box<dyn Future<Output = core::result::Result<Self, Self::Error>> + 'a>> {
    Box::pin(<T as FromRequest<'a>>::from_request(req))
  }
}

macro_rules! impl_local_handler {
    ($($T:ident),+ $(,)?) => {
        impl<Func, Fut, R, $($T,)*> LocalHandler<($($T,)*)> for Func
        where
            Func: FnOnce($($T),*) -> Fut + Clone + 'static,
            Fut: Future<Output = R> + 'static,
            R: Responder,
            $( $T: Extract, )*
        {
            fn call(self, mut req: Request) -> impl Future<Output = Response> + 'static {
                async move {
                    $(
                        let $T = match <$T as Extract>::extract(&mut req).await {
                            Ok(value) => value,
                            Err(err) => return err.into_response(),
                        };
                    )*
                    (self)($($T),*).await.into_response()
                }
            }
        }
    };
}

impl_local_handler!(T1);
impl_local_handler!(T1, T2);
impl_local_handler!(T1, T2, T3);
impl_local_handler!(T1, T2, T3, T4);
impl_local_handler!(T1, T2, T3, T4, T5);
impl_local_handler!(T1, T2, T3, T4, T5, T6);
impl_local_handler!(T1, T2, T3, T4, T5, T6, T7);
impl_local_handler!(T1, T2, T3, T4, T5, T6, T7, T8);
impl_local_handler!(T1, T2, T3, T4, T5, T6, T7, T8, T9);
impl_local_handler!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
impl_local_handler!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);
impl_local_handler!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12);
