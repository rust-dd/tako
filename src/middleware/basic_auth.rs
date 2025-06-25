use crate::{
    body::TakoBody,
    middleware::Next,
    responder::Responder,
    types::{Request, Response},
};
use base64::Engine;
use http::{HeaderValue, StatusCode, header};
use std::{collections::HashMap, marker::PhantomData, pin::Pin, sync::Arc};

pub struct Config<U, F>
where
    F: Fn(&str, &str) -> Option<U> + Send + Sync + 'static,
    U: Send + Sync + 'static,
{
    users: Option<Arc<HashMap<String, String>>>,
    verify: Option<Arc<F>>,
    realm: &'static str,
    _phantom: PhantomData<U>,
}

impl<U, F> Config<U, F>
where
    F: Fn(&str, &str) -> Option<U> + Clone + Send + Sync + 'static,
    U: Clone + Send + Sync + 'static,
{
    pub fn single(user: impl Into<String>, pass: impl Into<String>) -> Self {
        Self {
            users: Some(Arc::new([(user.into(), pass.into())].into())),
            verify: None,
            realm: "Restricted",
            _phantom: PhantomData,
        }
    }

    pub fn multiple<I, S>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (S, S)>,
        S: Into<String>,
    {
        Self {
            users: Some(Arc::new(
                pairs
                    .into_iter()
                    .map(|(u, p)| (u.into(), p.into()))
                    .collect(),
            )),
            verify: None,
            realm: "Restricted",
            _phantom: PhantomData,
        }
    }

    pub fn with_verify(cb: F) -> Self {
        Self {
            users: None,
            verify: Some(Arc::new(cb)),
            realm: "Restricted",
            _phantom: PhantomData,
        }
    }

    pub fn realm(mut self, r: &'static str) -> Self {
        self.realm = r;
        self
    }

    pub fn into_middleware(
        self,
    ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
    + Clone
    + Send
    + Sync
    + 'static {
        let users = self.users;
        let verify = self.verify;
        let realm = self.realm;

        move |mut req: Request, next: Next| {
            let users = users.clone();
            let verify = verify.clone();

            Box::pin(async move {
                let creds = req
                    .headers()
                    .get(header::AUTHORIZATION)
                    .and_then(|h| h.to_str().ok())
                    .and_then(|h| h.strip_prefix("Basic "))
                    .and_then(|b64| base64::engine::general_purpose::STANDARD.decode(b64).ok())
                    .and_then(|raw| String::from_utf8(raw).ok())
                    .and_then(|s| s.split_once(':').map(|(u, p)| (u.to_owned(), p.to_owned())));

                match creds {
                    None => {}
                    Some((u, p)) => {
                        if let Some(map) = &users {
                            if map.get(&u).map(|pw| pw == &p).unwrap_or(false) {
                                return next.run(req).await.into_response();
                            }
                        }

                        if let Some(cb) = &verify {
                            if let Some(obj) = cb(&u, &p) {
                                req.extensions_mut().insert(obj);
                                return next.run(req).await.into_response();
                            }
                        }
                    }
                }

                let mut res = Response::new(TakoBody::empty());
                *res.status_mut() = StatusCode::UNAUTHORIZED;
                res.headers_mut().append(
                    header::WWW_AUTHENTICATE,
                    HeaderValue::from_str(&format!("Basic realm=\"{realm}\"")).unwrap(),
                );
                res
            })
        }
    }
}
