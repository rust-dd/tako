use std::{pin::Pin, sync::Arc};

use anyhow::Result;
use http::request::Parts;

use crate::{
    responder::Responder,
    types::{BoxedResponseFuture, Request, Response},
};

pub trait FromRequest<S, M = ()>: Sized {
    type Rejection: Responder;

    fn from_request(
        req: Request,
        state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send;
}

pub trait FromRequestParts<S>: Sized {
    type Rejection: Responder;

    fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send;
}

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

// macro_rules! impl_handler {
//     ([$($ty:ident),*], $last:ident) => {
//         #[allow(non_snake_case, unused_mut)]
//         impl<F, Fut, S, R, M, $($ty,)* $last> Handler<(M, $($ty,)* $last,), S> for F
//         where
//             F: FnOnce($($ty,)* $last,) -> Fut + Clone + Send + Sync + 'static,
//             Fut: Future<Output = R> + Send,
//             S: Send + Sync + 'static,
//             R: Responder,
//             $( $ty: FromRequestParts<S> + Send, )*
//             $last: FromRequest<S, M> + Send,
//         {
//             type Future = Pin<Box<dyn Future<Output = Response> + Send>>;

//             fn call(self, req: Request, state: S) -> Self::Future {
//                 let (mut parts, body) = req.into_parts();
//                 Box::pin(async move {
//                     $(
//                         let $ty = match <$ty as FromRequestParts<S>>::from_request_parts(&mut parts, &state).await {
//                             Ok(value) => value,
//                             Err(err) => return err.into_response(),
//                         };
//                     )*

//                     let req = Request::from_parts(parts, body);
//                     let $last = match <$last as FromRequest<S, M>>::from_request(req, &state).await {
//                         Ok(value) => value,
//                         Err(err) => return err.into_response(),
//                     };

//                     (self)($($ty,)* $last).await.into_response()
//                 })
//             }
//         }
//     };
// }

// impl_handler!([], T1);
// impl_handler!([T1], T2);
// impl_handler!([T1, T2], T3);
// impl_handler!([T1, T2, T3], T4);
// impl_handler!([T1, T2, T3, T4], T5);
// impl_handler!([T1, T2, T3, T4, T5], T6);
// impl_handler!([T1, T2, T3, T4, T5, T6], T7);
// impl_handler!([T1, T2, T3, T4, T5, T6, T7], T8);

#[derive(Clone)]
pub struct BoxedHandler {
    inner: Arc<dyn Fn(Request) -> BoxedResponseFuture + Send + Sync>,
}

impl BoxedHandler {
    pub(crate) fn new<H>(h: H) -> Self
    where
        H: Handler + Clone,
    {
        let inner = Arc::new(move |req: Request| {
            let handler = h.clone();
            Box::pin(async move { handler.call(req.into()).await }) as BoxedResponseFuture
        });

        Self { inner }
    }

    pub(crate) fn call(&self, req: Request) -> BoxedResponseFuture {
        (self.inner)(req)
    }
}
