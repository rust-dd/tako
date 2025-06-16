use std::{pin::Pin, sync::Arc};

use anyhow::Result;
use http::request::Parts;

use crate::{
    responder::Responder,
    types::{BoxedHandlerFuture, Request, Response},
};

pub trait FromRequest<S, M = ()>: Sized {
    type Rejection: Responder;

    fn from_request(
        req: Request,
        state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send;
}

pub struct Test;
pub struct Test1;
pub struct Test2;

impl<S, M> FromRequest<S, M> for Test {
    type Rejection = ();

    fn from_request(
        _req: Request,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        async { Ok(Test) }
    }
}

impl<S> FromRequestParts<S> for Test1 {
    type Rejection = ();

    fn from_request_parts(
        _req: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        async { Ok(Test1) }
    }
}

impl<S> FromRequestParts<S> for Test2 {
    type Rejection = ();

    fn from_request_parts(
        _req: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        async { Ok(Test2) }
    }
}

pub trait FromRequestParts<S>: Sized {
    type Rejection: Responder;

    fn from_request_parts(
        req: &mut Parts,
        state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send;
}

pub trait Handler<T, S>: Send + Sync + 'static {
    type Future: Future<Output = Response> + Send + 'static;

    fn call(self, _req: Request, _state: S) -> Self::Future;
}

impl<F, Fut, R, S> Handler<((),), S> for F
where
    F: FnOnce() -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = R> + Send,
    R: Responder,
{
    type Future = Pin<Box<dyn Future<Output = Response> + Send>>;

    fn call(self, _req: Request, _state: S) -> Self::Future {
        Box::pin(async move { (self)().await.into_response() })
    }
}

macro_rules! impl_handler {
    ([$($ty:ident),*], $last:ident) => {
        #[allow(non_snake_case, unused_mut)]
        impl<F, Fut, S, R, M, $($ty,)* $last> Handler<(M, $($ty,)* $last,), S> for F
        where
            F: FnOnce($($ty,)* $last,) -> Fut + Clone + Send + Sync + 'static,
            Fut: Future<Output = R> + Send,
            S: Send + Sync + 'static,
            R: Responder,
            $( $ty: FromRequestParts<S> + Send, )*
            $last: FromRequest<S, M> + Send,
        {
            type Future = Pin<Box<dyn Future<Output = Response> + Send>>;

            fn call(self, req: Request, state: S) -> Self::Future {
                let (mut parts, body) = req.into_parts();
                Box::pin(async move {
                    $(
                        let $ty = match <$ty as FromRequestParts<S>>::from_request_parts(&mut parts, &state).await {
                            Ok(value) => value,
                            Err(err) => return err.into_response(),
                        };
                    )*

                    let req = Request::from_parts(parts, body);
                    let $last = match <$last as FromRequest<S, M>>::from_request(req, &state).await {
                        Ok(value) => value,
                        Err(err) => return err.into_response(),
                    };

                    (self)($($ty,)* $last).await.into_response()
                })
            }
        }
    };
}

impl_handler!([], T1);
impl_handler!([T1], T2);
impl_handler!([T1, T2], T3);
impl_handler!([T1, T2, T3], T4);
impl_handler!([T1, T2, T3, T4], T5);
impl_handler!([T1, T2, T3, T4, T5], T6);
impl_handler!([T1, T2, T3, T4, T5, T6], T7);
impl_handler!([T1, T2, T3, T4, T5, T6, T7], T8);

#[derive(Clone)]
pub struct BoxedHandler<S> {
    inner: Arc<dyn Fn(Request, S) -> BoxedHandlerFuture + Send + Sync>,
    _phantom: std::marker::PhantomData<fn() -> S>,
}

impl<S> BoxedHandler<S> {
    pub(crate) fn new<H, T>(h: H) -> Self
    where
        H: Handler<T, S> + Clone,
        S: Send + Sync + 'static,
    {
        let inner = Arc::new(move |req: Request, state: S| {
            let handler = h.clone();
            Box::pin(async move { handler.call(req.into(), state).await }) as BoxedHandlerFuture
        });

        Self {
            inner,
            _phantom: std::marker::PhantomData,
        }
    }

    pub(crate) fn call(&self, req: Request, state: S) -> BoxedHandlerFuture {
        (self.inner)(req, state)
    }
}
