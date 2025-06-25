use http::{StatusCode, header};
use std::{collections::HashSet, future::Future, pin::Pin, sync::Arc};

use crate::{
    middleware::Next,
    responder::Responder,
    types::{Request, Response},
};

pub struct Config<C, F>
where
    F: Fn(&str) -> Option<C> + Send + Sync + 'static,
    C: Send + Sync + 'static,
{
    tokens: Option<HashSet<String>>,
    verify: Option<F>,
    _phantom: std::marker::PhantomData<C>,
}

impl<C, F> Config<C, F>
where
    F: Fn(&str) -> Option<C> + Clone + Send + Sync + 'static,
    C: Clone + Send + Sync + 'static,
{
    pub fn static_token(tok: impl Into<String>) -> Self {
        Self {
            tokens: Some([tok.into()].into()),
            verify: None,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn static_tokens<I>(toks: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        Self {
            tokens: Some(toks.into_iter().map(Into::into).collect()),
            verify: None,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn with_verify(f: F) -> Self {
        Self {
            tokens: None,
            verify: Some(f),
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn static_tokens_with_verify<I>(toks: I, f: F) -> Self
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        Self {
            tokens: Some(toks.into_iter().map(Into::into).collect()),
            verify: Some(f),
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn into_middleware(
        self,
    ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
    + Clone
    + Send
    + Sync
    + 'static {
        let tokens = self.tokens.map(Arc::new);
        let verify = self.verify.map(Arc::new);

        move |mut req: Request, next: Next| {
            let tokens = tokens.clone();
            let verify = verify.clone();

            Box::pin(async move {
                let tok = req
                    .headers()
                    .get(header::AUTHORIZATION)
                    .and_then(|h| h.to_str().ok())
                    .and_then(|h| h.strip_prefix("Bearer "))
                    .map(str::trim);

                match tok {
                    None => {}
                    Some(t) => {
                        if let Some(set) = &tokens {
                            if set.contains(t) {
                                return next.run(req).await.into_response();
                            }
                        }
                        if let Some(v) = verify.as_ref() {
                            if let Some(claims) = v(t) {
                                req.extensions_mut().insert(claims);
                                return next.run(req).await.into_response();
                            }
                        }
                    }
                }

                (
                    StatusCode::UNAUTHORIZED,
                    [(header::WWW_AUTHENTICATE, "Bearer")],
                )
                    .into_response()
            })
        }
    }
}
